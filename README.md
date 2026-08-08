# Terminal

基于 Rust 的 GPU 加速终端模拟器，使用 [alacritty_terminal](https://github.com/alacritty/alacritty) 处理终端仿真，[egui](https://github.com/emilk/egui) + wgpu 负责渲染。

## 功能

- GPU 加速渲染（wgpu 后端）
- CJK 支持（宽字符渲染、IME 输入、字体回退）
- 10,000 行滚动历史，智能滚动条
- 超链接识别，点击打开
- 文本选择（单词/行/自定义），选中即复制
- 正则搜索
- 光标闪烁
- 标签页 + 水平/垂直分屏
- 会话持久化（标签页、工作目录）

## 快捷键

- `Ctrl+C` — 复制选中文本（无选区时发送中断信号）
- `Ctrl+V` — 粘贴

## 构建与运行

```bash
cargo build --release
cargo run --release
```

## 环境要求

- Windows
- Rust 1.70+
- 支持 wgpu 的显卡
