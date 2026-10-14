# clutch

Global key bindings that take priority over every program. clutch grabs
the keyboards, reads every key, and passes all of them through except
the ones you have bound, which run a command instead. It works below the
desktop, so the same settings apply on X11, on Wayland and on the
console, regardless of the desktop's own shortcuts.

```toml
[bind]
"ctrl+space"    = "tilix --quake"
"print"         = "hotshot"
"super+space t" = "xterm"        # Super+Space, then T
```

## Install

```sh
cargo install --git https://github.com/jeffjose/clutch
clutch setup
```

`clutch setup` asks for sudo once, to install a udev rule that lets the
logged-in user read the machine's keyboards. It then writes
`~/.config/clutch/config.toml` with no bindings, and starts clutch as a
systemd user service. Nothing runs as root.

## Use

```sh
clutch status    # show what is running, which keyboards it holds, and what is bound
clutch pause     # release the keyboards until `clutch resume`
clutch check     # read the settings and report any problems
clutch remove    # uninstall clutch from this machine
```

The settings are reloaded whenever the file is saved. If the file has a
mistake, the existing bindings stay in effect. A key is written with its
modifiers (`ctrl`, `shift`, `alt`, `super`). For a sequence, write the
keys in order with a space between them.

A key that clutch handles is not seen by anything else. A desktop
shortcut on the same key does nothing while clutch is running, and works
again when it isn't.

## The box

![The box, listing things to do](docs/palette.png)

```toml
"super+k" = "clutch palette"
```

The box is a command palette. Type to list the matching windows and
applications, and press Enter to switch to the selected one. With
nothing typed, it lists your windows with the previously used one first,
so the shortcut followed by Enter switches back to it. A prefix narrows
what is searched:

| | |
|---|---|
| `>` | actions: move the front window to the next monitor, maximize or close it, send it to a workspace; run a binding; pause clutch |
| `@` | windows only |
| `#` | workspaces |
| `!` | a shell command |

An editor window is treated as a coding project. With nothing typed, the
projects are shown as tiles with a letter, the project's name, its git
status and its language. None of this needs configuring: the project
name comes from the window's title, the letter is the first unused one
in the name, and `Cargo.toml` or `package.json` provides the rest. Press
Alt and the letter to jump straight to a project. For example, clutch
itself is `Super+K`, then `Alt+C`.

Two more commands are meant to be bound to keys of their own:

```sh
clutch raise firefox       # switch to Firefox's window, or start Firefox
clutch move-monitor        # move the front window to the next monitor
```

None of these commands need clutch to be running. They start, do one
thing and exit, so a desktop shortcut can run them too.

On X11, any program can list and move windows. On Wayland only the
desktop can, so on GNOME `clutch setup` also installs a small shell
extension. The extension lists and moves windows for clutch, and centers
the box on screen with keyboard focus. It needs no sudo, but takes
effect after you log out and back in. Without it, and on other Wayland
desktops, the box offers only applications, bindings and shell commands.

## Without sudo

If the udev rule can't be installed, clutch can't grab the keyboards. On
GNOME it registers your bindings as GNOME custom shortcuts instead, and
keeps them in sync with the settings file. The file and the commands
stay the same. `clutch status` shows which mode a machine is in, and
which bindings couldn't be carried over:

- Only chords are supported, because GNOME has no key sequences.
- A GNOME shortcut on the same keys takes priority. `clutch status`
  prints the command that clears it.

Shortcuts you created in GNOME yourself are left alone.

## If it goes wrong

clutch sits between you and every key, so there are three ways out:

- **Hold Backspace, Escape and Enter together** to stop it, whatever
  else is happening.
- If it stops responding, systemd kills it within five seconds.
- However it ends, the kernel returns the keyboards.

clutch reads every key you press and stores none of them. The only
thing it logs is which bindings ran.

## Not yet

- Tap versus hold, and layers.
- Keyboards that are also a mouse or a touchpad. These are left alone.
- Bindings still run while the screen is locked.

## Tests

```sh
cargo test                # the logic
cargo test -- --ignored   # clutch itself, between a virtual keyboard and
                          # a reader; needs `clutch setup` first
```

## License

MIT
