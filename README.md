<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo-dark.svg?v=2">
    <img src="docs/logo.svg?v=2" alt="serialX" width="480">
  </picture>
</p>

一款基于 [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 与
[GPUI Component](https://github.com/longbridge/gpui-component) 的现代串口调试工作台。

## 当前能力

- 自动发现本机串口设备，并支持完整串口参数配置、连接、断开和后台读取
- 从 `Session` 菜单、标题栏的 `New` 按钮或 ⌘N 打开新建会话对话框，每个标签保留自己的连接和终端内容
- 右侧面板可持久化串口会话，并提供可保存、复用和删除的快捷发送命令
- 右侧面板两级折叠：单击分区标题折叠单个分区，⌘B 或标题栏右端的开关把整个面板收成带数量角标的图标栏
- 终端下方的输入栏集成 ASCII / HEX 分段开关，支持 Enter 快速发送
- 终端上方只有一条标签栏：左侧是会话标签（连接状态点、端口名、关闭），右端直接提供连接 / 断开、暂停接收、清屏、时间戳与自动滚动；刷新端口在 `Session` 菜单里
- 纯白与近黑两套工作台主题，默认以近黑主题启动，可在 `View` 菜单中切换
- 启动时检查 GitHub Releases，支持校验并安装最新版本
- 三段式标题栏（菜单栏）：左侧是当前会话的上下文胶囊，显示端口、参数与连接状态，点开即可在已打开
  和已保存的会话之间切换；中央是命令中心，左右箭头切换标签，过滤框按正则表达式或纯文本筛选终端输出，
  可切换大小写敏感，右端实时显示“匹配行数 / 总行数”，写错的表达式会标红提示而不会隐藏任何输出；
  右侧只留一个侧边面板开关
- 右侧面板的左边缘可以拖动，在 220 到 560 像素之间调整宽度，折叠成图标栏再展开时宽度保持不变
- 新建会话对话框以“选择”而非“表单”呈现：设备是一张可滚动的单选列表，波特率是一个可直接键入的下拉框（列表里是标准速率，也接受任意自定义速率），
  数据位、校验、停止位与流控是四组分段开关，底部实时给出 `115200 8N1` 式摘要与逐项说明
- 自定义 macOS 标题栏与紧凑、低干扰的编辑器式工作台布局
- 参考 Material Icon Theme 的双色圆角图标集：设备、会话、命令、信号各有专属色相
- 统一的排版比例，字体选择与 VS Code 对齐：不打包字库，按平台复用 VS Code 的字族栈，启动时取本机
  第一款已安装的字族。终端沿用编辑器字族（macOS Menlo、Windows Consolas、Linux Droid Sans Mono），
  界面里的等宽文本沿用 VS Code 的 `--monaco-monospace-font`（macOS SF Mono / Monaco），界面字体则是
  各平台的系统 UI 字体或 Segoe UI
- 按系统语言挂载 CJK 回退字族（PingFang SC、Microsoft YaHei、Source Han Sans 等），设备发来的
  中日韩文本照常显示，读者自己的语言排在最前

## 运行

```bash
cargo run
```

首次构建需要下载 GPUI 相关依赖，耗时会稍长。建议使用最新稳定版 Rust；macOS
还需要完整的 Xcode / Command Line Tools 环境。

## 下载

GitHub Releases 提供以下预编译包：

- macOS Apple Silicon：DMG
- Windows x86_64：安装程序与便携 ZIP
- Linux x86_64 / ARM64：DEB 与便携 tar.gz

当前发布包尚未进行 Apple 公证或 Windows 代码签名，首次启动时系统可能显示安全提示。

## 软件更新

serialX 启动后会在后台检查仓库中最新的正式 GitHub Release；发现新版本时会弹出
更新提示。也可以随时通过 `Help > Check for Updates…` 手动检查，检查完成后会明确
提示当前已是最新版或提供“下载并安装”操作。应用会下载当前系统对应的安装包，使用
Release 附带的 SHA-256 摘要校验文件完整性，再打开系统安装程序。版本、许可证和
项目地址可在 `Help > About serialX` 中查看：

- macOS：打开 DMG，按窗口提示替换“应用程序”中的 serialX
- Windows：启动安装程序并退出当前 serialX，以便替换正在使用的程序文件
- Linux：打开 DEB 安装包，由系统软件安装器完成升级

自动检查只读取公开 Release 信息，不需要 GitHub 登录或访问令牌；草稿版和预发布版
不会被视为可用更新。

发布新版本前，使用版本升级脚本同步更新 `Cargo.toml` 与 `Cargo.lock`：

```bash
scripts/version-bump.sh 0.2.0
```

脚本会校验版本号，创建 `chore: bump version to 0.2.0` 提交并推送当前分支；
Release 工作流会从 `Cargo.toml` 读取该版本并创建对应的 `v0.2.0` 标签。

## 快捷操作

- `Enter`：发送当前内容
- `⌘N` / `Ctrl+N`：新建会话；`⌘S` / `Ctrl+S` 保存当前会话；`⌘W` / `Ctrl+W` 关闭当前会话
- `⌘⇧C` / `Ctrl+Shift+C`：连接 / 断开；`⌘R` / `Ctrl+R` 重新扫描端口
- `⌘⇧P` / `Ctrl+Shift+P`：暂停 / 恢复接收；`⌘K` / `Ctrl+K` 清空终端
- `⌘⇧H` / `Ctrl+Shift+H`：HEX 显示；`⌘⇧T` / `Ctrl+Shift+T` 时间戳；`⌘⇧A` / `Ctrl+Shift+A` 自动滚动
- `⌘F` / `Ctrl+F`：聚焦标题栏的输出过滤框，`Esc` 清空过滤
- `⌘⇧[` / `⌘⇧]`（`Ctrl+PageUp` / `Ctrl+PageDown`）：切换到左侧 / 右侧标签
- `⌘B` / `Ctrl+B`：显示或隐藏右侧面板；`⌘⇧L` / `Ctrl+Shift+L` 切换亮暗主题
- `Session` 菜单：新建、保存或关闭会话，连接、重新扫描端口、暂停接收、清空终端，切换上一个 / 下一个会话
- `View` 菜单：输出过滤、HEX 显示、时间戳、自动滚动、侧边面板，以及 `Appearance` 子菜单中的亮暗主题
- 标题栏左侧会话胶囊：单击列出已打开与已保存的会话，可直接切换或打开
- 拖动右侧面板的左边缘调整面板宽度
- 标题栏过滤框：`.*` 切换正则表达式（默认开启），`Aa` 切换大小写敏感，× 清空
- 终端工具栏：连接 / 断开、重新扫描端口、暂停接收、清空终端、时间戳、自动滚动
- 新建会话对话框：在设备列表中单选端口并可随时 `Rescan`，波特率芯片、数据位 / 校验 / 停止位 / 流控分段开关，
  底部实时显示配置摘要，`Enter` 确认、`Esc` 取消
- 右侧 `Sessions`：保存或恢复会话配置，右键已保存项可重新编辑；已在标签中打开的端口会显示绿点
- 右侧 `Quick send`：单击发送已保存的 AT 命令；输入栏右侧的书签按钮可把当前输入存为新命令
- 单击分区标题可折叠该分区；折叠后的面板保留图标栏，单击图标即可展开对应分区

## 图标版权

serialX 应用图标及 `assets/icons/` 下的衍生图标资源由 miskin 设计，
版权所有 © 2026 miskin，并与本项目一致采用 GNU GPL v3 授权。
