# Installation

```bash
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh
```

The installer works on Linux (x86-64 and ARM, glibc) and macOS (Apple silicon and Intel). It
installs into `~/.femp` (or `FEMP_HOME`), adds `~/.femp/bin` to `PATH` in your shell's
startup files, and sets up everything `femp` runs:

| Component | Used for |
| --- | --- |
| `femp`, with the Fe 26.6.0 compiler and its EVM, native and WebAssembly backends built in | Every command. There is no separate `fe` to install. |
| Z3, in `~/.femp/z3` | `femp prove`. Its Linux builds need glibc 2.38 or newer; on older systems femp uses a `z3` from the system's package manager. |
| Node.js, in `~/.femp/node`, when no Node.js 20 or newer is on `PATH` | Running WebAssembly. It is kept for femp alone and does not change which `node` other programs see. |
| A C compiler, when none is found | Linking native executables: `gcc` from the system's package manager on Linux, Apple's Command Line Tools on macOS. The installer asks first, because this may need your password. |

Every download is checked against a SHA-256 list. Options go after `sh -s --`:

```bash
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh -s -- --yes
```

| Option | Effect |
| --- | --- |
| `--yes` | Install a missing C compiler without asking, for scripts and CI. |
| `--no-modify-path` | Leave shell startup files alone. |
| `--skip-deps` | Install femp and Z3 only. |

`FEMP_VERSION=v0.1.0` installs a specific release. To uninstall, delete `~/.femp` and the line
the installer added to `~/.profile`, `~/.bashrc`, `~/.bash_profile` or `~/.zshrc`.

## Checking the installation

`femp doctor` shows the linked compiler, the tools femp found and which platforms this machine
can build. On a Mac set up by the installer:

```text
Compiler
  fe         26.6.0 (5b3f96a), linked into femp
  backends   sonatina, native, wasm
Host
  system     aarch64 macos
  node       v24 (/usr/local/bin/node)
  cc         found
  z3         5.4.1 (/Users/you/.femp/z3/bin/z3)
Platforms
  evm        stable        available
  native     experimental  available
  wasm       experimental  available
```

Native executables build on x86-64 Linux and Apple silicon. When a platform is unavailable,
`femp doctor` says why and how to fix it, and other commands skip it with a warning unless you
name it with `-t`.

## Building from source

Building from source needs Rust 1.97 or newer. Cargo fetches the Fe compiler's crates at the
pinned release, so the first build takes a few minutes longer.

```bash
git clone https://github.com/emeieiron/femp.git && cd femp
```

```bash
cargo install --path crates/femp-cli
```

Running what femp builds needs Node.js 20 or newer, a C compiler, and Z3 for `femp prove`.
Point `FEMP_NODE` at a particular Node.js, and `FEMP_Z3` at a particular Z3. Without
`FEMP_NODE`, femp uses a new enough `node` on `PATH`, then the one under `FEMP_HOME`. Without
`FEMP_Z3`, it uses the Z3 under `FEMP_HOME`, then `z3` on `PATH`.

## Editor support

FeMP projects are ordinary Fe ingots, so the official Fe language server, formatter and
documentation generator work on them unchanged. Install the `fe` toolchain alongside FeMP for
`fe lsp`, `fe fmt` and `fe doc`, and run [`femp generate`](source-generators.md) so the editor
also sees generated sources.
