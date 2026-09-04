//! Supplies the application icon, both to the running process and to the
//! workbench itself.
//!
//! Packaged builds get their icon from the platform bundle: `serialX.app`
//! points `CFBundleIconFile` at `SerialX.icns` on macOS, and `build.rs` embeds
//! `SerialX.ico` into the Windows executable. `cargo run` on macOS launches the
//! bare executable instead of the bundle, so there is no `Info.plist` to read
//! and the Dock falls back to the generic executable icon. Handing the icon to
//! `NSApplication` gives development builds the same Dock and switcher icon the
//! packaged app ships with.

use std::sync::{Arc, OnceLock};

use gpui_kit::{Image, ImageFormat};

/// Sets the Dock and switcher icon for the current process.
///
/// Must be called on the main thread; it is a no-op anywhere else, and on
/// platforms where the icon already comes from the executable or a desktop
/// entry.
pub fn apply_application_icon() {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;

        const ICON: &[u8] = include_bytes!("../assets/icons/macos/SerialX.icns");

        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        let Some(icon) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(ICON)) else {
            return;
        };

        // SAFETY: `setApplicationIconImage:` takes any `NSImage`, and the
        // main thread marker proves we are on the thread that owns `NSApp`.
        unsafe {
            NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&icon));
        }
    }
}

/// The application icon, decoded once for in-app rendering.
///
/// The window content cannot reach the platform icon, so the same artwork the
/// bundles ship is embedded here and handed to `img`.
pub fn application_icon_image() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();

    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/icons/png/serialx-256.png").to_vec(),
        ))
    })
    .clone()
}
