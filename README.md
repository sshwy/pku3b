# PKU3b: A Better Black Board for PKUers

pku3b 是一个由 Rust 实现的简单命令行工具，用于爬取北京大学教学网的信息。它的用法如下：

```
A tool for PKU students to check their courses.

Usage: pku3b [COMMAND]

Commands:
  assignment  获取作业信息 [aliases: a]
  video       获取课程回放 [aliases: v]
  init        (重新) 初始化配置选项
  config      显示或修改配置项
  clean       清除缓存
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Motivation

众所周知 PKU 的教学网 UI 长得非常次时代，信息获取效率奇低。对此已有的解决方案是借助 [PKU-Art](https://github.com/zhuozhiyongde/PKU-Art) 把 UI 变得赏心悦目一点。

但是如果你和我一样已经进入到早十起不来、签到不想管、不知道每天要上什么课也不想关心、对教学网眼不见为净的状态，那我猜你至少会关注作业的 DDL，或者期末的时候看看回放。于是 pku3b 应运而生。