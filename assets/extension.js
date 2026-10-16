    // The windows of the desktop, for clutch. On Wayland only the shell
    // can see and move them, so this small part of clutch lives in it and
    // answers over D-Bus. It also puts clutch's own box where it belongs
    // and gives it the keyboard, which a window cannot ask for itself.

    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, '/io/github/jeffjose/Clutch');
        this._mapped = global.window_manager.connect('map', (_wm, actor) => {
            this._place(actor.meta_window);
            this._visorMapped(actor.meta_window);
        });
        this._focused = global.display.connect('notify::focus-window', () => this._visorFocused());
    }

    disable() {
        global.window_manager.disconnect(this._mapped);
        global.display.disconnect(this._focused);
        this._visor = null;
        this._dbus.unexport();
        this._dbus = null;
    }

    _ours(window) {
        return window.get_wm_class() === 'clutch';
    }

    // The windows a taskbar would show, the one used last first.
    _windows() {
        const all = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        return all.filter(window => !window.is_skip_taskbar() && !this._ours(window) && !this._isVisor(window));
    }

    // The shell numbers its windows from somewhere at random, which can be
    // past what clutch counts to. The end of the number tells them apart.
    _id(window) {
        return window.get_id() % 4294967296;
    }

    _find(id) {
        return this._windows().find(window => this._id(window) === id);
    }

    // The box, in the middle of the monitor in use each way, over
    // everything, with the keyboard.
    _place(window) {
        if (!window || !this._ours(window))
            return;
        const area = global.display.get_monitor_geometry(global.display.get_current_monitor());
        const frame = window.get_frame_rect();
        window.move_frame(true, area.x + Math.floor((area.width - frame.width) / 2), area.y + Math.max(0, Math.floor((area.height - frame.height) / 2)));
        window.make_above();
        Main.activateWindow(window);
    }

    // Everything the box lists, as JSON in plain ASCII.
    State() {
        const manager = global.workspace_manager;
        const tracker = Shell.WindowTracker.get_default();
        const front = global.display.focus_window;
        const windows = this._windows().map(window => {
            const app = tracker.get_window_app(window);
            return {
                id: this._id(window),
                title: window.get_title() ?? '',
                program: app?.get_name() ?? window.get_wm_class() ?? '',
                icon: app?.get_icon()?.to_string() ?? '',
                workspace: window.is_on_all_workspaces() ? -1 : window.get_workspace().index(),
                front: window === front,
            };
        });
        const workspaces = [];
        for (let at = 0; at < manager.get_n_workspaces(); at++)
            workspaces.push(Meta.prefs_get_workspace_name(at));
        const state = {
            version: VERSION,
            windows,
            workspaces,
            workspace: manager.get_active_workspace_index(),
            monitors: global.display.get_n_monitors(),
        };
        const plain = letter => '\\u' + letter.charCodeAt(0).toString(16).padStart(4, '0');
        return JSON.stringify(state).replace(/[\u007f-￿']/g, plain);
    }

    Activate(id) {
        const window = this._find(id);
        if (window)
            Main.activateWindow(window);
    }

    Close(id) {
        this._find(id)?.delete(global.get_current_time());
    }

    Maximize(id) {
        const window = this._find(id);
        if (!window)
            return;
        // How this is asked changed between versions of the shell.
        const is = window.is_maximized ? window.is_maximized() : window.get_maximized() === Meta.MaximizeFlags.BOTH;
        try {
            if (is)
                window.unmaximize(Meta.MaximizeFlags.BOTH);
            else
                window.maximize(Meta.MaximizeFlags.BOTH);
        } catch (_) {
            if (is)
                window.unmaximize();
            else
                window.maximize();
        }
    }

    Fullscreen(id) {
        const window = this._find(id);
        if (window?.is_fullscreen())
            window.unmake_fullscreen();
        else
            window?.make_fullscreen();
    }

    MoveToMonitor(id, back) {
        const window = this._find(id);
        const count = global.display.get_n_monitors();
        if (window && count > 1)
            window.move_to_monitor((window.get_monitor() + (back ? count - 1 : 1)) % count);
    }

    SendTo(id, workspace) {
        this._find(id)?.change_workspace_by_index(workspace, false);
    }

    GoTo(workspace) {
        global.workspace_manager.get_workspace_by_index(workspace)?.activate(global.get_current_time());
    }

    // visor: a terminal of a class of its own, brought in from an edge
    // of the screen and sent away again. clutch says where, in parts of
    // the screen, and what to do: toggle, show, hide, or only say. The
    // answer is where it is then: shown, hidden, or absent, when there
    // is no such window and clutch is to start one. That one is put in
    // place as it comes up.
    Visor(asked) {
        const want = JSON.parse(asked);
        this._visor = want;
        const window = this._visorWindow();
        if (!window) {
            this._visorPending = want.do === 'toggle' || want.do === 'show';
            return 'absent';
        }
        const shown = !window.minimized;
        if (want.do === 'state')
            return shown ? 'shown' : 'hidden';
        const away = want.do === 'hide' || (want.do === 'toggle' && shown && global.display.focus_window === window);
        if (away) {
            if (shown)
                this._visorHide(window);
            return 'hidden';
        }
        this._visorShow(window, !shown);
        return 'shown';
    }

    _isVisor(window) {
        return !!this._visor && window.get_wm_class() === this._visor.class;
    }

    _visorWindow() {
        const windows = global.get_window_actors().map(actor => actor.meta_window);
        return windows.find(window => this._isVisor(window)) ?? null;
    }

    _visorMapped(window) {
        if (!window || !this._visorPending || !this._isVisor(window))
            return;
        this._visorPending = false;
        this._visorShow(window, false);
    }

    // Away, when another window takes the keyboard, if clutch says so.
    // The box coming up over it does not count.
    _visorFocused() {
        const focus = global.display.focus_window;
        if (!this._visor?.blur || !focus || this._isVisor(focus) || this._ours(focus))
            return;
        const window = this._visorWindow();
        if (window && !window.minimized && focus.get_transient_for() !== window)
            this._visorHide(window);
    }

    _visorShow(window, slide) {
        const want = this._visor;
        const display = global.display;
        const monitor = want.main ? display.get_primary_monitor() : display.get_current_monitor();
        const actor = window.get_compositor_private();
        if (window.minimized) {
            if (actor)
                Main.wm.skipNextEffect(actor);
            window.unminimize();
        }
        if (window.is_fullscreen())
            window.unmake_fullscreen();
        const maximized = window.get_maximized ? window.get_maximized() !== 0 : window.is_maximized();
        if (maximized) {
            try {
                window.unmaximize(Meta.MaximizeFlags.BOTH);
            } catch (_) {
                window.unmaximize();
            }
        }
        if (window.get_monitor() !== monitor)
            window.move_to_monitor(monitor);
        const area = window.get_work_area_for_monitor(monitor);
        const [x, y, w, h] = want.rect;
        const gap = want.gap ?? 0;
        const left = area.x + Math.round(x * area.width) + gap;
        const top = area.y + Math.round(y * area.height) + gap;
        const width = Math.max(1, Math.round(w * area.width) - 2 * gap);
        const height = Math.max(1, Math.round(h * area.height) - 2 * gap);
        window.move_resize_frame(true, left, top, width, height);
        if (!window.is_above())
            window.make_above();
        if (!window.is_on_all_workspaces())
            window.stick();
        Main.activateWindow(window);
        if (slide && want.slide && actor)
            this._visorSlide(actor, want.edge, width, height, true, null);
    }

    _visorHide(window) {
        const actor = window.get_compositor_private();
        const away = () => {
            if (actor) {
                actor.translation_x = 0;
                actor.translation_y = 0;
                actor.opacity = 255;
                Main.wm.skipNextEffect(actor);
            }
            window.minimize();
        };
        if (!this._visor?.slide || !actor)
            return away();
        const frame = window.get_frame_rect();
        this._visorSlide(actor, this._visor.edge, frame.width, frame.height, false, away);
    }

    // Slide the terminal in from off its edge, or out to it; from the
    // middle, it fades.
    _visorSlide(actor, edge, width, height, coming, done) {
        const off = {top: [0, -height], bottom: [0, height], left: [-width, 0], right: [width, 0]}[edge] ?? [0, 0];
        const fade = off[0] === 0 && off[1] === 0 ? 0 : 255;
        const out = [off[0], off[1], fade];
        const [from, to] = coming ? [out, [0, 0, 255]] : [[0, 0, 255], out];
        actor.remove_all_transitions();
        actor.translation_x = from[0];
        actor.translation_y = from[1];
        actor.opacity = from[2];
        actor.ease({
            translation_x: to[0],
            translation_y: to[1],
            opacity: to[2],
            duration: 160,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            onStopped: () => done?.(),
        });
    }
