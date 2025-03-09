# PKU3b: A Better Black Board for PKUers

[![Crates.io](https://img.shields.io/crates/v/pku3b)](https://crates.io/crates/pku3b)

pku3b 是一个由 Rust 实现的简单命令行工具，用于爬取北京大学教学网的信息。目前它可以

- [x] 查看课程作业信息（未完成/全部）
- [x] 下载课程作业附件
- [ ] 提交课程作业
- [x] 查看课程回放列表
- [x] 下载课程回放（需要 ffmpeg）

查看作业/下载附件:

![demo-a](assets/demo-pku3b-a.gif)

基本用法如下：

```text
A tool for PKU students to check their courses.

Usage: pku3b [COMMAND]

Commands:
  assignment  获取作业信息 [aliases: a]
  video       获取课程回放 [aliases: v]
  init        (重新) 初始化配置选项
  config      显示或修改配置项
  cache       查看缓存大小/清除缓存
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Getting Started

该工具目前只在 MacOS 上测试过，按理支持 Linux，Windows 目前尚不支持。你需要使用 cargo 安装/更新:

```bash
cargo install pku3b
```

如果需要使用下载课程回放的功能 (使用 `pku3b help v down` 查看用法)，你需要安装 `ffmpeg`。在 MacOS 上可以使用 Homebrew 安装: `brew install ffmpeg`.

首次执行命令前你需要登陆教学网。执行以下命令，然后根据提示输入教学网账号密码来完成初始化设置（只需要执行一次，如果想要更改配置，只需要再执行一次；如果想要进行细粒度的修改，也可以使用 `pku3b config <key> <value>` 修改配置项）:

```bash
pku3b init
```

示例：

- 查看未完成的作业列表: `pku3b a ls`
- 查看全部作业列表: `pku3b a ls -a`
- 下载作业附件: `pku3b a down <ID>` (ID 请在作业列表中查看)
- 查看课程回放列表: `pku3b v ls`
- 下载课程回放: `pku3b v down <ID>` (ID 请在课程回放列表中复制)
- 查看缓存占用: `pku3b cache`
- 清空缓存: `pku3b cache clean`
- 查看某个命令的使用方法 (以下载课程回放的命令为例): `pku3b help v down`

## Motivation

众所周知 PKU 的教学网 UI 长得非常次时代，信息获取效率奇低。对此已有的解决方案是借助 [PKU-Art](https://github.com/zhuozhiyongde/PKU-Art) 把 UI 变得赏心悦目一点。

但是如果你和我一样已经进入到早十起不来、签到不想管、不知道每天要上什么课也不想关心、对教学网眼不见为净的状态，那我猜你至少会关注作业的 DDL，或者期末的时候看看回放。于是 pku3b 应运而生。
