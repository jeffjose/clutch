    // The windows of the desktop, for clutch. On Wayland only the shell
    // can see and move them, so this small part of clutch lives in it and
    // answers over D-Bus. It also puts clutch's own box where it belongs
    // and gives it the keyboard, which a window cannot ask for itself.

    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, '/io/github/jeffjose/Clutch');
        this._mapped = global.window_manager.connect('map', (_wm, actor) => this._place(actor.meta_window));
    }

    disable() {
        global.window_manager.disconnect(this._mapped);
        this._dbus.unexport();
        this._dbus = null;
    }

    _ours(window) {
        return window.get_wm_class() === 'clutch';
    }

    // The windows a taskbar would show, the one used last first.
    _windows() {
        const all = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        return all.filter(window => !window.is_skip_taskbar() && !this._ours(window));
    }

    _find(id) {
        return this._windows().find(window => window.get_id() === id);
    }

    // The box, across the middle of the monitor in use and a fifth of
    // the way down, over everything, with the keyboard.
    _place(window) {
        if (!window || !this._ours(window))
            return;
        const area = global.display.get_monitor_geometry(global.display.get_current_monitor());
        const frame = window.get_frame_rect();
        window.move_frame(true, area.x + Math.floor((area.width - frame.width) / 2), area.y + Math.floor(area.height / 5));
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
                id: window.get_id(),
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
