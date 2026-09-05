# Video Quality Test Tool

Measure how much quality a video loses when you compress it. Give the tool a reference file
and one or more encodes. It measures them frame by frame, draws one graph for each metric,
and lets you look at the worst frames.

**The tool always gives you the number.** When your files disagree about color range, size or
frame count, the tool corrects what it safely can, runs, and then tells you what it changed.
It never refuses to measure.

The tool is a single window. It has no command line, no folder batch and no run history in
version 1.0.

## Programs used

The tool ships no back end and downloads nothing. Get the programs you want, then point the
tool at them in Settings. Only the first row is required.

| Program | Metrics | Source |
| --- | --- | --- |
| FFmpeg and ffprobe | PSNR, SSIM, XPSNR, VMAF, CAMBI, and the media information of every file | [ffmpeg.org](https://ffmpeg.org/download.html) |
| The VMAF models | VMAF v1, which needs a model file that matches your resolution | [github.com/Netflix/vmaf](https://github.com/Netflix/vmaf) |
| FFVship | SSIMULACRA 2, Butteraugli and ColorVideoVDP, on a supported graphics card | [codeberg.org/Line-fr/Vship](https://codeberg.org/Line-fr/Vship) |
| ssimulacra2_rs | SSIMULACRA 2 on a machine with no supported graphics card | [github.com/rust-av/ssimulacra2_bin](https://github.com/rust-av/ssimulacra2_bin) |

FFmpeg must be version 7.1 or later and must be built with `--enable-libvmaf`. A build made
with `--enable-gpl` also works, but this tool only decodes, so it does not need one.

The first time you start the tool with no back end found, it opens Settings and shows this
list with a link for each program. Press **find** on a row after you install it.

## Install

Unpack the archive for your system and run `vqtt`. There is no installer, and it needs no
administrator rights. The tool writes your settings to the configuration folder of your
operating system and to nowhere else.

| System | Settings |
| --- | --- |
| Windows | `%APPDATA%\vqtt` |
| macOS | `~/Library/Application Support/vqtt` |
| Linux | `$XDG_CONFIG_HOME/vqtt`, or `~/.config/vqtt` |

### Exporting

The tool writes one folder for each run, named `vqtt-<run id>`.

| File | Holds |
| --- | --- |
| `run.json` | Every setting that changed a number, the back-end versions, and the file fingerprints |
| `frames-<encode>.csv` | One row for each frame, one column for each metric, one time axis |
| `summary.csv` | One row for each encode and metric, with the pooled values |
| `commands.txt` | Every command line that ran, with its exit code and how long it took |
| `graph-<metric>.svg` and `.png` | The graph of each metric, the same picture as the screen |

`frames-<encode>.csv` opens in a spreadsheet. It is the file to keep if you want to pool the
values a different way later.

## Build from source

Rust 1.88 or later.

```
cargo build --release -p vqtt-gui
```

The binary lands at `target/release/vqtt`. The workspace holds four crates: `vqtt-core` is
pure logic, `vqtt-backends` builds the command lines and parses the logs, `vqtt-run` runs the
processes and writes the records, and `vqtt-gui` is the window. Only the last one depends on
a window library, so the tests of the first three need no display.

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

The programs that this tool calls are not part of it, and they carry their own licenses. The
tool starts each of them as a separate process and links none of them. Refer to
[THIRD-PARTY](THIRD-PARTY) for the code that is built into the binary.

### Contribution

Unless you state otherwise, any contribution you send for inclusion in this work, as defined
in the Apache-2.0 license, will be licensed as above, with no additional terms or conditions.
Refer to [CONTRIBUTING.md](CONTRIBUTING.md).
