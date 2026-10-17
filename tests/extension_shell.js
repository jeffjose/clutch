// A made-up GNOME shell, just enough of one to run clutch's extension
// (the variant from before modules, which reads imports.gi) and drive
// visor through it. `cargo test` runs it, where there is node.
'use strict';
const fs = require('fs');

const source = fs.readFileSync(process.argv[2], 'utf8');
let failures = 0;
const check = (what, ok) => {
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`);
    if (!ok) failures++;
};

// ---- settings, as dconf holds them ----
const SETTINGS = {
    'org.gnome.mutter.keybindings': {'toggle-tiled-left': ['<Super>Left'], 'toggle-tiled-right': ['<Super>Right']},
    'org.gnome.desktop.wm.keybindings': {'maximize': ['<Super>Up'], 'unmaximize': ['<Super>Down', '<Alt>F5'], 'close': ['<Alt>F4']},
    'org.gnome.shell.keybindings': {'toggle-overview': ['<Super>s']},
    'org.gnome.mutter.wayland.keybindings': {'restore-shortcuts': ['<Super>Escape']},
};
const ORIGINAL = JSON.parse(JSON.stringify(SETTINGS));
const schemaOf = id => ({
    id,
    list_keys: () => Object.keys(SETTINGS[id]),
    get_key: () => ({get_value_type: () => ({dup_string: () => 'as'})}),
});
class Settings {
    constructor({settings_schema, schema_id}) { this.id = settings_schema ? settings_schema.id : schema_id; }
    get_strv(key) { return [...SETTINGS[this.id][key]]; }
    set_strv(key, value) { SETTINGS[this.id][key] = [...value]; }
}

// ---- files ----
const FILES = new Map();
const GLib = {
    PRIORITY_DEFAULT: 0, SOURCE_REMOVE: false,
    getenv: name => (name === 'XDG_STATE_HOME' ? '/state' : null),
    get_home_dir: () => '/home/u',
    build_filenamev: parts => parts.join('/'),
    path_get_dirname: path => path.slice(0, path.lastIndexOf('/')),
    mkdir_with_parents: () => 0,
    file_set_contents: (path, text) => { FILES.set(path, text); return true; },
    file_get_contents: path => {
        if (!FILES.has(path))
            throw new Error('no such file');
        return [true, new TextEncoder().encode(FILES.get(path))];
    },
    unlink: path => { FILES.delete(path); return 0; },
    timers: [],
    timeout_add: (_p, _ms, f) => { GLib.timers.push(f); return GLib.timers.length; },
    source_remove: () => {},
};
const runTimers = () => {
    while (GLib.timers.length)
        GLib.timers.shift()();
};

// ---- processes ----
const started = [];
class SubprocessLauncher {
    constructor() { this.env = {}; }
    setenv(name, value) { this.env[name] = value; }
    set_cwd(dir) { this.cwd = dir; }
}
let client = null;
const Gio = {
    DBusExportedObject: {wrapJSObject: () => ({export() {}, unexport() {}})},
    DBus: {session: {}},
    SettingsSchemaSource: {get_default: () => ({lookup: id => (SETTINGS[id] ? schemaOf(id) : null)})},
    Settings,
    SubprocessLauncher,
    SubprocessFlags: {NONE: 0},
    Subprocess: {new: argv => started.push(argv)},
};

// ---- windows ----
let nextId = 1;
class Signals {
    constructor() { this.handlers = new Map(); this.ids = 0; }
    connect(name, f) { const id = ++this.ids; this.handlers.set(id, [name, f]); return id; }
    disconnect(id) { this.handlers.delete(id); }
    emit(name, ...args) {
        for (const [n, f] of [...this.handlers.values()])
            if (n === name)
                f(this, ...args);
    }
}
class Actor {
    constructor() { this.translation_x = 0; this.translation_y = 0; this.opacity = 255; this.eased = []; }
    remove_all_transitions() {}
    ease(params) {
        this.eased.push(params);
        this.translation_x = params.translation_x;
        this.translation_y = params.translation_y;
        this.opacity = params.opacity;
        params.onStopped?.(true);
    }
}
class Window extends Signals {
    constructor(klass) {
        super();
        this.klass = klass;
        this.id = nextId++;
        this.minimized = false;
        this.frame = {x: 100, y: 100, width: 800, height: 600};
        this.actor = new Actor();
        this.above = false;
        this.sticky = false;
        this.monitor = 0;
    }
    get_wm_class() { return this.klass; }
    get_id() { return this.id; }
    minimize() { this.minimized = true; this.emit('notify::minimized'); if (display.focus_window === this) display.setFocus(null); }
    unminimize() { this.minimized = false; this.emit('notify::minimized'); }
    is_fullscreen() { return false; }
    get_maximized() { return 0; }
    get_monitor() { return this.monitor; }
    move_to_monitor(m) { this.monitor = m; }
    get_work_area_for_monitor(m) { return m === 0 ? {x: 0, y: 32, width: 1920, height: 1048} : {x: 1920, y: 0, width: 2560, height: 1440}; }
    move_resize_frame(_user, x, y, width, height) { this.frame = {x, y, width, height}; }
    get_frame_rect() { return this.frame; }
    is_above() { return this.above; }
    make_above() { this.above = true; }
    is_on_all_workspaces() { return this.sticky; }
    stick() { this.sticky = true; }
    get_compositor_private() { return this.actor; }
    get_transient_for() { return null; }
    is_skip_taskbar() { return false; }
    get_title() { return this.klass; }
    get_workspace() { return {index: () => 0}; }
    delete() {}
}
const windows = [];

// ---- the display, and its keys ----
const grabs = new Map();
let nextAction = 100;
const normal = accelerator => {
    const mods = new Set();
    let rest = accelerator;
    let match;
    while ((match = /^<([^>]+)>/.exec(rest))) {
        mods.add(match[1].toLowerCase());
        rest = rest.slice(match[0].length);
    }
    return [...mods].sort().join('+') + '+' + rest.toLowerCase();
};
class Display extends Signals {
    constructor() { super(); this.focus_window = null; this.current = 0; }
    setFocus(window) { this.focus_window = window; this.emit('notify::focus-window'); }
    get_primary_monitor() { return 0; }
    get_current_monitor() { return this.current; }
    get_n_monitors() { return 2; }
    get_tab_list() { return windows; }
    // The shell's own keys keep theirs: an accelerator one of them has
    // cannot be grabbed.
    grab_accelerator(accelerator) {
        const taken = Object.values(SETTINGS).some(keys => Object.values(keys).some(list => list.some(a => normal(a) === normal(accelerator))));
        if (taken)
            return 0;
        const action = nextAction++;
        grabs.set(action, accelerator);
        return action;
    }
    ungrab_accelerator(action) { grabs.delete(action); return true; }
}
const display = new Display();
const windowManager = new Signals();

globalThis.global = {
    display,
    window_manager: windowManager,
    context: {},
    get_window_actors: () => windows.map(window => ({meta_window: window})),
    get_current_time: () => 0,
    workspace_manager: {get_n_workspaces: () => 1, get_active_workspace_index: () => 0},
};
const Meta = {
    is_wayland_compositor: () => true,
    WaylandClient: {
        new_subprocess: (_context, launcher, argv) => {
            client = {launcher, argv, owned: [], hidden: [], owns_window: w => client.owned.includes(w), hide_from_window_list: w => client.hidden.push(w)};
            return client;
        },
    },
    KeyBindingFlags: {NONE: 0},
    KeyBindingAction: {NONE: 0},
    external_binding_name_for_action: action => `external-grab-${action}`,
    MaximizeFlags: {BOTH: 3},
    TabList: {NORMAL_ALL: 0},
    prefs_get_workspace_name: () => 'Workspace 1',
};
const allowed = new Map();
globalThis.imports = {
    gi: {Clutter: {AnimationMode: {EASE_OUT_QUAD: 1}}, Gio, GLib, Meta, Shell: {ActionMode: {NONE: 0, NORMAL: 1, OVERVIEW: 2}, WindowTracker: {get_default: () => ({get_window_app: () => null})}}},
    ui: {main: {activateWindow: w => display.setFocus(w), wm: {skipNextEffect() {}, allowKeybinding: (name, mode) => allowed.set(name, mode)}}},
};
globalThis.log = () => {};

// ---- the extension ----
const load = () => new Function(`${source}\nreturn init();`)();
let ext = load();
ext.enable();

const ask = (what, extra = {}) => ext.Visor(JSON.stringify(Object.assign({
    do: what, class: 'io.github.jeffjose.visor', rect: [0, 0, 1, 0.4], edge: 'top', gap: 0, main: false, slide: true, blur: false,
    keys: [['<Super>Left', 'left'], ['<Super>Right', 'right'], ['<Super>Up', 'top'], ['<Super>Down', 'bottom'], ['<Super>g', 'grow'], ['<Super>Escape', 'hide']],
    clutch: '/home/u/.cargo/bin/clutch',
}, extra)));

// 1. Nothing yet: clutch is told, then asks the shell to start it.
check('no terminal: absent', ask('toggle') === 'absent');
check('the shell starts it', ask('start', {run: ['/bin/sh', '-c', 'exec ghostty -e screen -dRR'], path: '/home/u/bin:/usr/bin'}) === 'starting');
check('  with its command and PATH', client && client.argv[2] === 'exec ghostty -e screen -dRR' && client.launcher.env.PATH === '/home/u/bin:/usr/bin');

// It comes up.
const visor = new Window('io.github.jeffjose.visor');
windows.push(visor);
client.owned.push(visor);
windowManager.emit('map', {meta_window: visor});
check('left out of the window list', client.hidden.includes(visor));
check('put at the top, 40% of the work area', JSON.stringify(visor.frame) === JSON.stringify({x: 0, y: 32, width: 1920, height: 419}));
check('above and on every workspace, with the keyboard', visor.above && visor.sticky && display.focus_window === visor);
runTimers();
check('its six keys taken', grabs.size === 6);
check('  GNOME\'s tiling keys set aside', SETTINGS['org.gnome.mutter.keybindings']['toggle-tiled-left'].length === 0 && SETTINGS['org.gnome.desktop.wm.keybindings'].unmaximize.join() === '<Alt>F5');
check('  and written down', FILES.has('/state/clutch/visor-lent.json'));
check('  others left as they were', SETTINGS['org.gnome.desktop.wm.keybindings'].close.join() === '<Alt>F4');

// A key, pressed.
const left = [...grabs.entries()].find(([, a]) => a === '<Super>Left')[0];
display.emit('accelerator-activated', left);
check('Super+Left runs clutch visor left', JSON.stringify(started.at(-1)) === JSON.stringify(['/home/u/.cargo/bin/clutch', 'visor', 'left']));
check('  allowed in the shell', allowed.get(`external-grab-${left}`) === 3);

// Moved by clutch: a show with another rectangle.
check('show to the right third', ask('show', {rect: [0.66, 0, 0.34, 1], edge: 'right'}) === 'shown' && visor.frame.x === 1267 && visor.frame.height === 1048);

// 2. Toggled while it has the keyboard: away, sliding out to its edge.
visor.actor.eased = [];
check('toggle with the keyboard: hidden', ask('toggle', {rect: [0.66, 0, 0.34, 1], edge: 'right'}) === 'hidden');
check('  slid out to the right', visor.actor.eased.length === 1 && visor.actor.eased[0].translation_x === visor.frame.width);
check('  minimized, put back unmoved', visor.minimized && visor.actor.translation_x === 0);
check('  its keys given back', grabs.size === 0);
check('  GNOME\'s keys back as they were', JSON.stringify(SETTINGS) === JSON.stringify(ORIGINAL) && !FILES.has('/state/clutch/visor-lent.json'));
check('state: hidden', ask('state') === 'hidden');

// 3. Toggled again: in from its edge.
visor.actor.eased = [];
check('toggle again: shown', ask('toggle', {rect: [0.66, 0, 0.34, 1], edge: 'right'}) === 'shown');
check('  slid in from the right', visor.actor.eased.length === 1 && visor.actor.eased[0].translation_x === 0 && !visor.minimized);
runTimers();
check('  keys taken again', grabs.size === 6);

// 4. In, but another window has the keyboard: toggling gives it back the keyboard.
const editor = new Window('code');
windows.push(editor);
display.setFocus(editor);
check('toggle when behind: brought to the front', ask('toggle') === 'shown' && display.focus_window === visor && !visor.minimized);

// 5. The shell stops while visor is in; the next one puts the keys back.
check('set aside before the shell stops', SETTINGS['org.gnome.mutter.keybindings']['toggle-tiled-left'].length === 0);
grabs.clear(); // a shell that stops takes its grabs and handlers with it
for (const thing of [display, windowManager, ...windows]) thing.handlers.clear();
ext = load();
ext.enable();
check('a new shell puts them back', JSON.stringify(SETTINGS) === JSON.stringify(ORIGINAL) && !FILES.has('/state/clutch/visor-lent.json'));

// 6. Going when another window is clicked, when asked to.
ask('show', {blur: true});
runTimers();
display.setFocus(editor);
check('click away hides it, with blur on', visor.minimized);
ask('show', {blur: false});
display.setFocus(editor);
check('  and not with blur off', !visor.minimized);

// 7. Closed while in: its keys go with it.
ask('show');
runTimers();
check('in, with keys', grabs.size === 6);
visor.emit('unmanaged');
windows.splice(windows.indexOf(visor), 1);
check('closed: keys given back, GNOME\'s too', grabs.size === 0 && JSON.stringify(SETTINGS) === JSON.stringify(ORIGINAL));
check('absent after', ask('state') === 'absent');

// 8. On a second monitor, where the pointer is.
const second = new Window('io.github.jeffjose.visor');
windows.push(second);
display.current = 1;
ask('show', {rect: [0, 0, 0.5, 1], edge: 'left'});
check('on the monitor in use', second.monitor === 1 && second.frame.x === 1920 && second.frame.width === 1280);
check('the box\'s window list leaves it out', JSON.parse(ext.State()).windows.every(w => w.title !== 'io.github.jeffjose.visor'));

console.log(failures ? `${failures} failed` : 'all passed');
process.exit(failures ? 1 : 0);
