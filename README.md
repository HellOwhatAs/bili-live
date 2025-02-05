# bili-live

<p align="left">
    <a href="https://crates.io/crates/bili-live"><img src="https://img.shields.io/crates/v/bili-live"></a>
    <a href="https://github.com/HellOwhatAs/bili-live/"><img src="https://img.shields.io/github/languages/top/HellOwhatAs/bili-live"></a>
</p>

https://www.bilibili.com/video/BV1iHbueDEaw/

## Install
### Precompiled Binarys
Precompiled binarys available at https://github.com/HellOwhatAs/bili-live/releases.

### Install with Cargo
```
cargo install bili-live
```

## Usage
```
A command line tool for starting and stopping live streams on bilibili.com, capable of providing the RTMP address and stream key for streaming software such as OBS.

Usage: bili-live [COMMAND]

Commands:
  status  check live room status
  start   start live
  stop    stop live
  clean   clean login data
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```
