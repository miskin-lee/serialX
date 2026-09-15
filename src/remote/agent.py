# serialX remote agent.
#
# Runs on the host serialX is connected to over SSH. The host's `sh` starts
# a loader in Python, and serialX sends this file's source down the channel
# for it; nothing is installed on the host. It talks frames on its stdin and
# stdout — the SSH channel — and does on the host what serialX would do at
# home: lists
# the serial devices, opens them and moves their bytes both ways, reads and
# writes the files serialX keeps (the workspace, the recordings, the files
# a transfer moves) and shows the folders those files live in.
#
# Standard library only, Python 3.6 or later, POSIX hosts (Linux, macOS,
# the BSDs). The protocol is the one `src/remote/mod.rs` speaks; the two
# are changed together.

import errno
import fcntl
import json
import os
import platform
import queue
import select
import socket
import struct
import sys
import termios
import threading

PROTOCOL = 1
MARK = b"\x00serialX-agent\x00"
HEADER = struct.Struct(">BII")

# What arrives from serialX.
HELLO = ord("H")
LIST_PORTS = ord("L")
OPEN_PORT = ord("O")
WRITE_PORT = ord("W")
CLOSE_PORT = ord("C")
READ_FILE = ord("R")
WRITE_FILE = ord("S")
LIST_DIR = ord("D")
MAKE_DIR = ord("M")

# What goes back.
HELLO_REPLY = ord("h")
PORTS_REPLY = ord("l")
OPENED = ord("o")
PORT_DATA = ord("d")
PORT_CLOSED = ord("c")
FILE_REPLY = ord("r")
WRITTEN_REPLY = ord("s")
DIR_REPLY = ord("f")
ERROR = ord("e")

stdin = sys.stdin.buffer
stdout = sys.stdout.buffer
stdout_lock = threading.Lock()


def send(kind, channel, payload=b""):
    """Writes one frame; a stdout that has gone is the end of the agent."""
    with stdout_lock:
        try:
            stdout.write(HEADER.pack(kind, channel, len(payload)) + payload)
            stdout.flush()
        except (BrokenPipeError, OSError):
            os._exit(0)


def send_json(kind, channel, value):
    send(kind, channel, json.dumps(value).encode("utf-8"))


def send_error(channel, message):
    send(ERROR, channel, str(message).encode("utf-8", "replace"))


def read_exact(count):
    chunks = []
    while count > 0:
        chunk = stdin.read(count)
        if not chunk:
            return None
        chunks.append(chunk)
        count -= len(chunk)
    return b"".join(chunks)


# --- The host ---------------------------------------------------------------


def xdg_user_dir(name, fallback):
    """A folder from `~/.config/user-dirs.dirs`, else the fallback under
    home when it exists, else home itself."""
    home = os.path.expanduser("~")
    config = os.path.join(
        os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config"),
        "user-dirs.dirs",
    )
    key = "XDG_%s_DIR" % name
    try:
        with open(config) as file:
            for line in file:
                line = line.strip()
                if line.startswith(key + "="):
                    value = line.split("=", 1)[1].strip().strip('"')
                    value = value.replace("$HOME", home)
                    if os.path.isdir(value):
                        return value
    except OSError:
        pass
    candidate = os.path.join(home, fallback)
    return candidate if os.path.isdir(candidate) else home


def workspace_path():
    home = os.path.expanduser("~")
    if sys.platform == "darwin":
        return os.path.join(home, "Library/Application Support/serialX/workspace.json")
    root = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config")
    return os.path.join(root, "serialX/workspace.json")


def host_info():
    path = workspace_path()
    workspace = None
    try:
        with open(path, "rb") as file:
            workspace = file.read().decode("utf-8", "replace")
    except OSError:
        pass
    try:
        import pwd
        user = pwd.getpwuid(os.getuid()).pw_name
    except (ImportError, KeyError, OSError):
        user = os.environ.get("USER") or os.environ.get("LOGNAME") or ""
    return {
        "version": PROTOCOL,
        "hostname": socket.gethostname(),
        "user": user,
        "home": os.path.expanduser("~"),
        "platform": sys.platform,
        "machine": platform.machine(),
        "workspace_path": path,
        "workspace": workspace,
        "documents": xdg_user_dir("DOCUMENTS", "Documents"),
        "downloads": xdg_user_dir("DOWNLOAD", "Downloads"),
        "ports": list_ports(),
    }


# --- Devices ----------------------------------------------------------------


def read_text(path):
    try:
        with open(path) as file:
            return file.read().strip()
    except OSError:
        return ""


def usb_device_dir(start):
    """The USB device a tty hangs off, found by walking up from the tty's
    device until a folder carries a vendor id."""
    node = os.path.realpath(start)
    for _ in range(6):
        if os.path.exists(os.path.join(node, "idVendor")):
            return node
        parent = os.path.dirname(node)
        if parent == node:
            break
        node = parent
    return None


def is_real_8250(path):
    """Linux registers a handful of `ttyS` ports whether or not the board
    has them; the ones that exist answer the serial ioctl with a type."""
    TIOCGSERIAL = 0x541E
    try:
        fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as error:
        # A port that cannot be opened for lack of rights is still a port;
        # the failure shows when it is opened for real.
        return error.errno in (errno.EACCES, errno.EBUSY)
    try:
        buffer = bytearray(72)
        fcntl.ioctl(fd, TIOCGSERIAL, buffer)
        return struct.unpack_from("i", buffer, 0)[0] != 0
    except OSError:
        return False
    finally:
        os.close(fd)


def list_ports_linux():
    ports = []
    root = "/sys/class/tty"
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return ports
    for name in names:
        device = os.path.join(root, name, "device")
        if not os.path.exists(device):
            continue
        path = "/dev/" + name
        if not os.path.exists(path):
            continue
        subsystem = os.path.basename(
            os.path.realpath(os.path.join(device, "subsystem"))
        )
        driver = os.path.basename(os.path.realpath(os.path.join(device, "driver")))
        usb = usb_device_dir(device)
        if usb is not None:
            product = read_text(os.path.join(usb, "product"))
            maker = read_text(os.path.join(usb, "manufacturer"))
            subtitle = product or "USB Serial"
            if maker and maker.lower() not in subtitle.lower():
                subtitle = "%s %s" % (maker, subtitle)
            ports.append({"name": path, "subtitle": subtitle, "kind": "usb"})
        elif name.startswith("ttyS"):
            if not is_real_8250(path):
                continue
            kind = "pci" if subsystem in ("pci", "pnp") else "unknown"
            ports.append({"name": path, "subtitle": "Serial Port", "kind": kind})
        elif name.startswith("rfcomm"):
            ports.append({"name": path, "subtitle": "Bluetooth Serial", "kind": "bluetooth"})
        elif subsystem in ("platform", "amba", "of_platform", "serial-base"):
            label = driver if driver and driver != "driver" else "UART"
            ports.append({"name": path, "subtitle": "%s · on-board UART" % label, "kind": "unknown"})
        elif subsystem == "pci":
            ports.append({"name": path, "subtitle": "PCI Serial", "kind": "pci"})
        elif name.startswith(("ttyAMA", "ttyTHS", "ttymxc", "ttySAC", "ttyO", "ttyMSM", "ttyAML")):
            ports.append({"name": path, "subtitle": "On-board UART", "kind": "unknown"})
    return ports


def list_ports_darwin():
    ports = []
    try:
        names = sorted(os.listdir("/dev"))
    except OSError:
        return ports
    for name in names:
        if not name.startswith("cu."):
            continue
        path = "/dev/" + name
        lower = name.lower()
        if "bluetooth" in lower:
            ports.append({"name": path, "subtitle": "Bluetooth Serial", "kind": "bluetooth"})
        elif "usb" in lower:
            ports.append({"name": path, "subtitle": "USB Serial", "kind": "usb"})
        else:
            ports.append({"name": path, "subtitle": "Serial Device", "kind": "unknown"})
    return ports


def list_ports_bsd():
    ports = []
    try:
        names = sorted(os.listdir("/dev"))
    except OSError:
        return ports
    for name in names:
        path = "/dev/" + name
        if name.startswith("cuaU") and "." not in name:
            ports.append({"name": path, "subtitle": "USB Serial", "kind": "usb"})
        elif name.startswith("cuau") and "." not in name:
            ports.append({"name": path, "subtitle": "Serial Port", "kind": "unknown"})
    return ports


def list_ports():
    try:
        if sys.platform.startswith("linux"):
            return list_ports_linux()
        if sys.platform == "darwin":
            return list_ports_darwin()
        return list_ports_bsd()
    except Exception:  # noqa: BLE001 — a listing that fails is an empty one
        return []


# --- Ports ------------------------------------------------------------------

PARITIES = {"none": 0, "odd": termios.PARENB | termios.PARODD, "even": termios.PARENB}
CHAR_SIZES = {5: termios.CS5, 6: termios.CS6, 7: termios.CS7, 8: termios.CS8}
CRTSCTS = getattr(termios, "CRTSCTS", 0o20000000000)


def set_custom_baud(fd, baud):
    """A rate the termios constants do not name."""
    if sys.platform.startswith("linux"):
        TCGETS2 = 0x802C542A
        TCSETS2 = 0x402C542B
        BOTHER = 0o10000
        CBAUD = 0o10017
        layout = struct.Struct("IIIIB19sII")
        buffer = bytearray(layout.size)
        fcntl.ioctl(fd, TCGETS2, buffer)
        iflag, oflag, cflag, lflag, line, cc, _, _ = layout.unpack_from(buffer)
        cflag = (cflag & ~CBAUD) | BOTHER
        layout.pack_into(buffer, 0, iflag, oflag, cflag, lflag, line, cc, baud, baud)
        fcntl.ioctl(fd, TCSETS2, buffer)
    elif sys.platform == "darwin":
        IOSSIOSPEED = 0x80085402
        fcntl.ioctl(fd, IOSSIOSPEED, struct.pack("Q", baud))
    else:
        raise OSError("this host cannot set a baud rate of %d" % baud)


def configure_port(fd, settings):
    baud = int(settings.get("baud", 115200))
    attrs = termios.tcgetattr(fd)
    iflag, oflag, cflag, lflag, ispeed, ospeed, cc = attrs
    iflag &= ~(
        termios.IGNBRK | termios.BRKINT | termios.PARMRK | termios.ISTRIP
        | termios.INLCR | termios.IGNCR | termios.ICRNL
        | termios.IXON | termios.IXOFF | termios.IXANY
    )
    oflag &= ~termios.OPOST
    lflag &= ~(termios.ECHO | termios.ECHONL | termios.ICANON | termios.ISIG | termios.IEXTEN)
    cflag &= ~(termios.CSIZE | termios.PARENB | termios.PARODD | termios.CSTOPB | CRTSCTS)
    cflag |= termios.CLOCAL | termios.CREAD
    cflag |= CHAR_SIZES.get(int(settings.get("data_bits", 8)), termios.CS8)
    cflag |= PARITIES.get(settings.get("parity", "none"), 0)
    if int(settings.get("stop_bits", 1)) == 2:
        cflag |= termios.CSTOPB
    flow = settings.get("flow", "none")
    if flow == "hardware":
        cflag |= CRTSCTS
    elif flow == "software":
        iflag |= termios.IXON | termios.IXOFF
    cc = list(cc)
    cc[termios.VMIN] = 0
    cc[termios.VTIME] = 0
    constant = getattr(termios, "B%d" % baud, None)
    if constant is not None:
        ispeed = ospeed = constant
    termios.tcsetattr(fd, termios.TCSANOW, [iflag, oflag, cflag, lflag, ispeed, ospeed, cc])
    if constant is None:
        set_custom_baud(fd, baud)
    termios.tcflush(fd, termios.TCIFLUSH)


class Port:
    """One open device: a thread reading it onto the channel, a thread
    writing what the channel brings, and the pipe that wakes the reader
    when the port is closed from this end."""

    def __init__(self, channel, path, settings):
        self.channel = channel
        self.path = path
        self.fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        try:
            configure_port(self.fd, settings)
        except Exception:
            os.close(self.fd)
            raise
        self.wake_r, self.wake_w = os.pipe()
        self.outgoing = queue.Queue()
        self.closed = threading.Event()
        self.lock = threading.Lock()
        self.reader = threading.Thread(target=self.read_loop, daemon=True)
        self.writer = threading.Thread(target=self.write_loop, daemon=True)
        self.reader.start()
        self.writer.start()

    def read_loop(self):
        reason = ""
        while not self.closed.is_set():
            try:
                ready, _, _ = select.select([self.fd, self.wake_r], [], [], 1.0)
            except (OSError, ValueError):
                reason = "the port went away"
                break
            if self.wake_r in ready:
                break
            if self.fd not in ready:
                continue
            try:
                data = os.read(self.fd, 4096)
            except BlockingIOError:
                continue
            except OSError as error:
                reason = "read failed: %s" % error.strerror
                break
            if not data:
                reason = "the device went away"
                break
            send(PORT_DATA, self.channel, data)
        self.finish(reason)

    def write_loop(self):
        while True:
            data = self.outgoing.get()
            if data is None:
                return
            view = memoryview(data)
            while view and not self.closed.is_set():
                try:
                    count = os.write(self.fd, view)
                    view = view[count:]
                except BlockingIOError:
                    select.select([self.wake_r], [self.fd], [], 1.0)
                except OSError as error:
                    self.finish("write failed: %s" % error.strerror)
                    return

    def write(self, data):
        self.outgoing.put(data)

    def close(self):
        self.finish("")

    def finish(self, reason):
        with self.lock:
            if self.closed.is_set():
                return
            self.closed.set()
        try:
            os.write(self.wake_w, b"x")
        except OSError:
            pass
        self.outgoing.put(None)
        try:
            os.close(self.fd)
        except OSError:
            pass
        with ports_lock:
            ports.pop(self.channel, None)
        send(PORT_CLOSED, self.channel, reason.encode("utf-8", "replace"))


ports = {}
ports_lock = threading.Lock()


def open_port(channel, payload):
    try:
        settings = json.loads(payload.decode("utf-8"))
        path = settings["path"]
        with ports_lock:
            if channel in ports:
                raise OSError("channel %d is already open" % channel)
            ports[channel] = Port(channel, path, settings)
        send(OPENED, channel)
    except Exception as error:  # noqa: BLE001 — every reason reaches the log
        message = describe(error)
        if isinstance(error, OSError) and error.errno == errno.EACCES:
            message += " (is the account in the group that owns the device?)"
        send_error(channel, message)


def describe(error):
    if isinstance(error, OSError) and error.strerror:
        return error.strerror
    return str(error) or error.__class__.__name__


# --- Files ------------------------------------------------------------------


def unique_path(folder, name):
    first = os.path.join(folder, name)
    if not os.path.exists(first):
        return first
    stem, dot, extension = name.rpartition(".")
    if not stem:
        stem, extension, dot = name, "", ""
    n = 2
    while True:
        candidate = os.path.join(folder, "%s (%d)%s%s" % (stem, n, dot, extension))
        if not os.path.exists(candidate):
            return candidate
        n += 1


def read_file(channel, payload):
    try:
        request = json.loads(payload.decode("utf-8"))
        with open(os.path.expanduser(request["path"]), "rb") as file:
            send(FILE_REPLY, channel, file.read())
    except Exception as error:  # noqa: BLE001
        send_error(channel, describe(error))


def write_file(channel, payload):
    try:
        (length,) = struct.unpack_from(">H", payload, 0)
        header = json.loads(payload[2 : 2 + length].decode("utf-8"))
        data = payload[2 + length :]
        path = os.path.expanduser(header["path"])
        folder = os.path.dirname(path)
        if header.get("mkdirs") and folder:
            os.makedirs(folder, exist_ok=True)
        if header.get("unique"):
            path = unique_path(folder, os.path.basename(path))
        mode = "ab" if header.get("append") else "wb"
        with open(path, mode) as file:
            file.write(data)
        send_json(WRITTEN_REPLY, channel, {"path": path, "size": len(data)})
    except Exception as error:  # noqa: BLE001
        send_error(channel, describe(error))


def list_dir(channel, payload):
    try:
        request = json.loads(payload.decode("utf-8"))
        path = os.path.expanduser(request.get("path") or "~")
        path = os.path.abspath(path)
        if not os.path.isdir(path):
            raise OSError("%s is not a folder" % path)
        entries = []
        for name in os.listdir(path):
            full = os.path.join(path, name)
            try:
                info = os.stat(full)
            except OSError:
                continue
            is_dir = os.path.isdir(full)
            entries.append({
                "name": name,
                "dir": is_dir,
                "size": 0 if is_dir else info.st_size,
                "hidden": name.startswith("."),
            })
        entries.sort(key=lambda entry: (not entry["dir"], entry["name"].lower()))
        parent = os.path.dirname(path)
        send_json(DIR_REPLY, channel, {
            "path": path,
            "parent": parent if parent != path else None,
            "entries": entries,
        })
    except Exception as error:  # noqa: BLE001
        send_error(channel, describe(error))


def make_dir(channel, payload):
    try:
        request = json.loads(payload.decode("utf-8"))
        path = os.path.abspath(os.path.expanduser(request["path"]))
        os.makedirs(path, exist_ok=True)
        send_json(WRITTEN_REPLY, channel, {"path": path, "size": 0})
    except Exception as error:  # noqa: BLE001
        send_error(channel, describe(error))


# --- The loop ---------------------------------------------------------------


def serve():
    # Whatever the login shell printed before the agent came up is not
    # ours; the mark is where the frames begin.
    with stdout_lock:
        stdout.write(MARK)
        stdout.flush()
    while True:
        header = read_exact(HEADER.size)
        if header is None:
            break
        kind, channel, length = HEADER.unpack(header)
        payload = read_exact(length) if length else b""
        if payload is None:
            break
        if kind == HELLO:
            send_json(HELLO_REPLY, channel, host_info())
        elif kind == LIST_PORTS:
            send_json(PORTS_REPLY, channel, list_ports())
        elif kind == OPEN_PORT:
            open_port(channel, payload)
        elif kind == WRITE_PORT:
            with ports_lock:
                port = ports.get(channel)
            if port is not None:
                port.write(payload)
        elif kind == CLOSE_PORT:
            with ports_lock:
                port = ports.get(channel)
            if port is not None:
                port.close()
            else:
                send(PORT_CLOSED, channel)
        elif kind == READ_FILE:
            threading.Thread(target=read_file, args=(channel, payload), daemon=True).start()
        elif kind == WRITE_FILE:
            write_file(channel, payload)
        elif kind == LIST_DIR:
            threading.Thread(target=list_dir, args=(channel, payload), daemon=True).start()
        elif kind == MAKE_DIR:
            make_dir(channel, payload)
        else:
            send_error(channel, "unknown frame %r" % chr(kind))
    with ports_lock:
        open_ports = list(ports.values())
    for port in open_ports:
        port.close()


if __name__ == "__main__":
    try:
        serve()
    except KeyboardInterrupt:
        pass
    os._exit(0)
