<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logo-dark.svg?v=2">
    <img src="docs/logo.svg?v=2" alt="serialX" width="480">
  </picture>
</p>

一款基于 [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 与
[GPUI Component](https://github.com/longbridge/gpui-component) 的现代串口调试工作台。

## 当前能力

- 自动发现本机串口设备，并支持连接、断开和后台读取
- ASCII / HEX 双模式收发，支持 Enter 快速发送
- 接收暂停、清屏、时间戳与实时流量统计
- 内置 Loopback 演示设备，没有硬件也能直接体验完整界面
- 自定义 macOS 标题栏与适合长时间使用的高对比工作台布局

## 运行

```bash
cargo run
```

首次构建需要下载 GPUI 相关依赖，耗时会稍长。建议使用最新稳定版 Rust；macOS
还需要完整的 Xcode / Command Line Tools 环境。

## 快捷操作

- `Enter`：发送当前内容
- `HEX`：按十六进制字节解析输入和展示终端数据
- 点击波特率：未连接时循环切换常用波特率
