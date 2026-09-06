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
administrator rights.

The tool writes to the folders that your operating system keeps for these things, and never
beside the program. The program can sit in a folder that you cannot write to.

| System | Settings | Cache and working files | Exports |
| --- | --- | --- | --- |
| Windows | `%APPDATA%\vqtt` | `%LOCALAPPDATA%\vqtt\cache` | `%USERPROFILE%\Documents\vqtt` |
| macOS | `~/Library/Application Support/vqtt` | `~/Library/Caches/vqtt` | `~/Documents/vqtt` |
| Linux | `$XDG_CONFIG_HOME/vqtt`, or `~/.config/vqtt` | `$XDG_CACHE_HOME/vqtt`, or `~/.cache/vqtt` | `~/Documents/vqtt` |

The cache holds what the tool can build again: the answer of the last back-end scan, and the
working files of a run under `runs`. The working files are the metric logs and every still
that the frame viewer made. The tool clears them when the next run starts.

Settings can name a different folder for the working files and for the exports.

### Exporting

Press **export** above the graph. The tool writes one folder for each run into the export
folder. It never stops to ask where to put it.

The folder carries the local date and time that the run started, as
`vqtt-2026-09-06T19-40-46.749+0800`. The offset at the end names the clock that read the
time, which keeps two runs apart when a local hour repeats. A system that does not give its
offset writes `Z` and the time in UTC. Inside `run.json` every moment is UTC, because a
moment that is kept must never be ambiguous.

| File | Holds |
| --- | --- |
| `run.json` | Every setting that changed a number, the back-end versions, and the file fingerprints |
| `frames-<encode>.csv` | One row for each frame, one column for each metric, one time axis |
| `summary.csv` | One row for each encode and metric, with the pooled values |
| `commands.txt` | Every command line that ran, with its exit code and how long it took |
| `graph-<metric>.svg` and `.png` | The graph of each metric, the same picture as the screen |

`frames-<encode>.csv` opens in a spreadsheet. It is the file to keep if you want to pool the
values a different way later.

## The frame viewer

Every metric has known faults, so a measurement ends with a person looking at the pixels.
Click a point on the graph. The viewer opens under the graph on the worst frame of the line
you clicked, and shows the reference, the encode, and the difference between them.

| Control | What it does |
| --- | --- |
| `gain` | Multiplies the difference, so a fault too small to see becomes visible |
| `1:1 ↗` | Opens the frames in their own window, at their own pixel size. A fitted image hides the small faults you are looking for |
| `wipe` | Puts the reference and the encode in one pane. Drag the line to move the split |
| `save PNG` | Writes the image you last clicked into the export folder. It starts on the encode |
| `← worse` and `better →` | Walk the frames in the order the metric puts them, worst first |

The three images sit on a flat gray that is the same in both themes, and no theme color ever
touches them. A colored surround changes how a person judges an image.

### The window

`1:1 ↗` opens a window of its own. You can move it to a second display and size it freely.
One image fills the window. Move the pointer to the lower edge to get the other images as
thumbnails, and click one to put it on the stage.

The zoom control gives **fit**, **50%**, **100% (1:1)**, **200%** and **400%**. An image
larger than the window pans when you drag it. The footer reads the size of the image and the
scale it is drawn at.

The window keeps the frame, the gain, the focus and the wipe position of the panel, and hands
them back when it closes. It closes on its own control and on `Esc`. It is not modal, so you
can click another point on the graph and the window follows.

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
