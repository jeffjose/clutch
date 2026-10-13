# clutch

Keys that are yours before any program's. clutch holds the keyboards,
reads every key, and passes on all but the ones you have bound, which
run a command instead. It sits below the desktop, so the same settings
work on X11, on Wayland and on the console, whatever the desktop's own
shortcuts say.

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

`clutch setup` asks for sudo once, to put in a udev rule that lets
whoever is logged in at the machine read its keyboards. Then it writes
`~/.config/clutch/config.toml`, with nothing bound, and starts clutch as
a systemd user service. Nothing runs as root.

## Use

```sh
clutch status    # what is running, the keyboards it holds, what is bound
clutch pause     # give the keyboards back, until `clutch resume`
clutch check     # read the settings and say what is wrong with them
clutch remove    # take clutch off this machine
```

The settings are read again whenever the file is saved; a mistake in
them leaves the bindings as they were. A key is written with its
modifiers (`ctrl`, `shift`, `alt`, `super`), and keys pressed one after
another with a space between them.

A key clutch takes is not seen by anything else, so a shortcut of the
desktop's on the same key does nothing while clutch runs, and works
again when it does not.

## If it goes wrong

clutch is between you and every key, so there are three ways out:

- **Backspace, Escape and Enter held together** stop it, whatever else
  is going on.
- If it stops answering, systemd kills it within five seconds.
- However it ends, the kernel gives the keyboards back.

It reads every key you press and keeps none: nothing is written down
but which bindings ran.

## Not yet

Tap against hold, layers, and keyboards that are also a mouse or a
touchpad, which are left alone. Bindings run while the screen is locked.

## Tests

```sh
cargo test                # the logic
cargo test -- --ignored   # clutch itself, between a keyboard that is not
                          # real and a reader; needs `clutch setup` first
```

## License

MIT
