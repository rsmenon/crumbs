# Crumbs

A light-weight terminal-based personal tasks, notes, and agenda manager, written in Rust.

This is loosely based on my hacky vim setup using custom plugins and shell scripts to capture and organize fleeting thoughts and ideas while in grad school -- now  modernized and redesigned in Ratatui with Claude Code.

As this is meant for my personal use, documentation is light/non-existant, but you can mostly get by with the navigational hints in the app. If you find it useful, send me a note and I'll add better documentation.

<img width="1470" height="923" src="https://github.com/user-attachments/assets/3e23cf8b-6634-4abc-80dd-085e338bcb0f" />


## Installation

Crumbs requires Rust and neovim. It uses your existing `init.lua` config. This has only been tested on macOS. I assume it should work on linux if you modify the `brew` commands accordingly, but it is untested. Alternate editors are also not supported. 

```bash
brew install rust neovim
cargo install --git https://github.com/rsmenon/crumbs.git
```

Launch it with `crumb`. Data is stored in `~/.crumb/`, and navigational key movements should be straightforward or accessible via the hint bar at the bottom or by pressing <kbd>?</kbd> for help.

