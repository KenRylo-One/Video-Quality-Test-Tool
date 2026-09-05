# Contributing

## Before you write code

Open an issue first for anything larger than a bug fix. The design of this tool is written
down, and a change that disagrees with it needs a decision, not a patch.

Two rules shape almost everything here.

1. **The tool always gives you the number.** There is no blocked state and no validity badge.
   When research says a comparison is wrong, that research must make the tool *act*, not make
   the tool complain.
2. **`vqtt-core`, `vqtt-backends` and `vqtt-run` hold no window code.** Their tests run with
   no display. This is what keeps a command line cheap to add later, and interface state
   leaking downward is the one change that makes it expensive.

## Before you send a patch

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

All four must pass. Continuous integration runs the same four on Windows, Linux and macOS.

Tests follow four rules.

1. No test asserts an exact metric score. A back-end version changes the third decimal place.
   Assert the direction, the order, the correction that fired, and the command line.
2. Assert the command line, not the number. It is the fastest test and it catches the worst
   defect.
3. Every correction has a test that fires it and a test that does not. A correction that
   never fires is the same defect as one that always fires.
4. No test needs a video file that this repository does not ship. A test that reads media
   from somewhere else passes on one machine and proves nothing on any other.

## The Developer Certificate of Origin

This project uses the Developer Certificate of Origin, version 1.1. It is the same one the
Linux kernel uses, and you can read it at [developercertificate.org](https://developercertificate.org/).

Signing off says that you wrote the change, or that you have the right to send it under the
license of this project. Add the line with `git commit -s`, which writes it for you.

```
Signed-off-by: Your Name <your.email@example.com>
```

Use your real name. This project asks for no contributor license agreement.

## Prose

Documentation in this repository is written in Simplified Technical English.

- Descriptive text: 25 words for each sentence. Procedural text: 20 words, imperative.
- Approved modals only: can, will and must.
- No contractions, no semicolons, and no Latin abbreviations.
- Every claim needs a source. Say plainly when a number is a community estimate and not a
  measurement.
