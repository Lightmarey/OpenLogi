import Gio from 'gi://Gio';
import Shell from 'gi://Shell';
import GLib from 'gi://GLib';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Scripting from 'resource:///org/gnome/shell/ui/scripting.js';

Gio._promisify(Gio.Subprocess.prototype, 'wait_check_async');
Gio._promisify(Gio.Subprocess.prototype, 'wait_async');

function check(condition, message) {
    if (!condition)
        throw new Error(message);
}

async function until(predicate, message) {
    for (let step = 0; step < 100; step++) {
        if (predicate())
            return;
        await Scripting.sleep(50);
    }
    throw new Error(message);
}

async function saveScreenshot(path) {
    if (!path)
        return;
    const file = Gio.File.new_for_path(path);
    const stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
    await new Shell.Screenshot().screenshot(false, stream);
    stream.close(null);
}

async function paintFrame() {
    await new Promise(resolve => {
        const signal = global.stage.connect('after-paint', () => {
            global.stage.disconnect(signal);
            resolve();
        });
        global.stage.queue_redraw();
    });
}

function selectedName(extension) {
    return [...extension._items.values()].find(item => item.tile.has_style_pseudo_class('checked'))?.name.text;
}

function brightness(color) {
    return (color.red + color.green + color.blue) / 3;
}

async function systemScheme(settings, scheme) {
    settings.set_string('color-scheme', `prefer-${scheme}`);
    await until(() => Main.getStyleVariant() === scheme, `Shell must adopt the ${scheme} system preference`);
    await paintFrame();
}

async function audioCommand(...args) {
    check(GLib.getenv('OPENLOGI_CROWN_TEST_AUDIO') === 'isolated', 'Audio mutations require the isolated test runner');
    const process = Gio.Subprocess.new(['pactl', ...args], Gio.SubprocessFlags.STDOUT_SILENCE);
    await process.wait_check_async(null);
    if (args[0] === 'set-default-sink') {
        const policy = Gio.Subprocess.new(['pw-metadata', '-n', 'default', '0', 'default.audio.sink',
            JSON.stringify({name: args[1]})], Gio.SubprocessFlags.STDOUT_SILENCE);
        await policy.wait_check_async(null);
    }
}

async function checkParameter(extension, text, fraction) {
    try {
        await until(() => extension._items.get('volume')?.value.text === text, `The HUD must display authoritative value ${text}`);
    } catch (error) {
        const control = extension._feedback._control;
        const sink = control.get_default_sink();
        print(`FEEDBACK DIAGNOSTIC: payload=${JSON.stringify(extension._view?.functions[0])}, control=${control.get_state()}, sink=${sink?.get_name()}, volume=${sink?.volume}, muted=${sink?.is_muted}, normal=${control.get_vol_max_norm()}, current=${extension._items.get('volume')?.value.text}`);
        throw error;
    }
    await paintFrame();
    const {tile, fill, value, icon} = extension._items.get('volume');
    check(value.visible && !icon.visible && fill.visible, 'A known parameter must show its value and fill instead of the icon');
    check(Math.abs(fill.width - tile.width * fraction) < 1, `Fill allocation must match ${fraction}: ${fill.width}/${tile.width}`);
    const [fillX, fillY] = fill.get_transformed_position();
    const [tileX, tileY] = tile.get_transformed_position();
    check(Math.abs(fillX - tileX) < 1 && Math.abs(fillY - tileY) < 1 && fill.height === tile.height,
        'Progress must split the whole tile vertically without displacing its content');
}

export const METRICS = {};

export async function run() {
    await Main.extensionManager._initializationPromise;
    const extension = Main.extensionManager.lookup('openlogi-crown@openlogi.dev')?.stateObj;
    check(extension, 'Crown extension must be enabled');
    const systemSettings = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
    await systemScheme(systemSettings, 'light');
    const appearanceFailures = [];
    const layoutFailures = [];
    const checkAppearance = (skin, scheme) => {
        const node = extension._actor.get_theme_node();
        const light = scheme === 'light';
        const foreground = node.get_foreground_color();
        const item = extension._items.values().next().value;
        const background = skin === 'logitech' ? item.tile.get_theme_node().get_background_color()
            : node.get_background_color();
        if (light ? brightness(foreground) >= 120 || brightness(background) <= 170
            : brightness(foreground) <= 180 || brightness(background) >= 130)
            appearanceFailures.push(`${skin}/${scheme}: foreground=${brightness(foreground)}, background=${brightness(background)}`);
        check(item.name.style.includes(`${foreground.red},${foreground.green},${foreground.blue}`),
            'Label foreground must refresh with its presentation theme');
        for (const {icon} of extension._items.values()) {
            const glyph = icon.get_theme_node().get_foreground_color();
            const svg = new TextDecoder().decode(icon.gicon.get_bytes().get_data());
            check(svg.includes(`rgba(${glyph.red},${glyph.green},${glyph.blue},${glyph.alpha / 255})`),
                'Rendered SVG bytes must refresh with the current icon foreground');
        }
    };
    await Scripting.createTestWindow({textInput: true});
    await Scripting.waitTestWindows();
    Main.overview.hide();
    await Scripting.sleep(300);
    const client = global.get_window_actors().map(actor => actor.meta_window)[0];
    check(client, 'GNOME test helper must map an application window');
    client.activate(global.get_current_time());
    await until(() => global.display.focus_window === client, 'Test application must acquire focus');
    const focused = global.display.focus_window;
    check(focused, 'A real application window must be focused before HUD presentation');
    const keyboardFocus = global.stage.get_key_focus();
    const monitor = focused.get_monitor();
    const area = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(monitor);
    const mapped = [];
    const misplaced = [];
    const addChrome = Main.layoutManager.addChrome;
    Main.layoutManager.addChrome = function (actor, options) {
        if (actor.has_style_class_name('openlogi-crown-hud')) {
            const checkPosition = () => {
                if (!actor.mapped || actor.opacity === 0)
                    return;
                const [, height] = actor.get_preferred_height(actor.width);
                if (Math.abs(actor.x + actor.width / 2 - area.x - area.width / 2) >= 2 ||
                    Math.abs(area.y + area.height - actor.y - height - 32) >= 2)
                    misplaced.push({x: actor.x, y: actor.y, width: actor.width, height,
                        margin: extension._settings.get_int('bottom-margin')});
            };
            actor.connect('notify::mapped', () => {
                if (!actor.mapped)
                    return;
                mapped.push(actor);
                checkPosition();
            });
            actor.connect('notify::opacity', checkPosition);
        }
        return addChrome.call(this, actor, options);
    };
    const subprocess = Gio.Subprocess.new([
        GLib.getenv('OPENLOGI_CROWN_TEST_BINARY'),
        'crown::gnome::tests::crown_hud_native_shell', '--exact', '--ignored', '--nocapture',
    ], Gio.SubprocessFlags.STDIN_PIPE);
    const advance = () => {
        subprocess.get_stdin_pipe().write_all(new Uint8Array([1]), null);
        subprocess.get_stdin_pipe().flush(null);
    };
    await until(() => extension._actor !== null, 'Rust presenter must create the Shell actor');
    await Scripting.sleep(150);
    check(extension._actor.visible && extension._actor.opacity === 255,
        'The initial HUD must become visible without further presentation updates');
    await paintFrame();
    check(global.display.focus_window === focused, 'HUD must retain application focus');
    check(global.stage.get_key_focus() === keyboardFocus, 'HUD must retain keyboard focus');
    check(!extension._actor.reactive && !extension._actor.can_focus, 'HUD must be read-only');
    const bounds = extension._actor;
    check(Math.abs(bounds.x + bounds.width / 2 - area.x - area.width / 2) < 2,
        'HUD must be horizontally centered on the application monitor');
    check(Math.abs(area.y + area.height - bounds.y - bounds.height - 32) < 2,
        'HUD must be anchored above the bottom of the work area');
    check(bounds.has_style_class_name('logitech'), 'The rectangular Logitech skin must be the default');
    check(bounds.get_first_child().has_style_class_name('openlogi-crown-row'), 'The HUD must omit the extra heading');
    check(bounds.width === 480, 'Three classic tiles must form a single rectangular strip');
    check(bounds.get_theme_node().get_border_radius(0) === 0, 'Classic skin must have square corners');
    checkAppearance('logitech', 'light');
    if (GLib.getenv('OPENLOGI_CROWN_TEST_AUDIO') === 'isolated')
        await checkParameter(extension, '25%', 0.25);
    for (const {tile, symbol, name} of extension._items.values()) {
        const [, tileY] = tile.get_transformed_position();
        const [, iconY] = symbol.get_transformed_position();
        const [, nameY] = name.get_transformed_position();
        const [, naturalHeight] = name.clutter_text.get_preferred_height(name.width);
        if (Math.abs((iconY + nameY + naturalHeight) / 2 - tileY - tile.height / 2) >= 2)
            layoutFailures.push('Logitech icon and natural label content must be centered together');
    }
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_LOGITECH_SCREENSHOT'));
    const owner = extension._owner;
    await systemScheme(systemSettings, 'dark');
    checkAppearance('logitech', 'dark');
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_LOGITECH_DARK_SCREENSHOT'));
    await systemScheme(systemSettings, 'light');
    checkAppearance('logitech', 'light');
    extension._settings.set_string('hud-theme', 'openlogi');
    await until(() => extension._actor && !extension._actor.has_style_class_name('logitech'),
        'Changing skin must refresh the visible HUD');
    extension._settings.set_int('bottom-margin', 64);
    await until(() => {
        const current = extension._actor;
        return current && Math.abs(area.y + area.height - current.y - current.height - 64) < 2;
    }, 'The configured bottom margin must apply to a visible HUD');
    check(extension._owner === owner, 'Skin and placement changes must retain the presenter lease');
    check(extension._actor === bounds, 'Skin and placement changes must retain the visible actor');
    check(global.display.focus_window === focused && global.stage.get_key_focus() === keyboardFocus,
        'Skin and placement changes must retain application and keyboard focus');
    await paintFrame();
    checkAppearance('openlogi', 'light');
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_OPENLOGI_SCREENSHOT'));
    await systemScheme(systemSettings, 'dark');
    checkAppearance('openlogi', 'dark');
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_OPENLOGI_DARK_SCREENSHOT'));
    await systemScheme(systemSettings, 'light');
    checkAppearance('openlogi', 'light');
    extension._settings.set_string('hud-theme', 'gnome');
    extension._settings.set_int('bottom-margin', 32);
    await until(() => {
        const current = extension._actor;
        return current?.has_style_class_name('popup-menu-content') &&
            Math.abs(area.y + area.height - current.y - current.height - 32) < 2;
    }, 'Restoring GNOME skin and placement must update the HUD');
    await paintFrame();
    checkAppearance('gnome', 'light');
    if (extension._actor.get_first_child().text === 'Volume')
        layoutFailures.push('GNOME skin must omit the redundant selected-function heading');
    if (extension._actor.width >= 336 || extension._actor.height > 100)
        layoutFailures.push('GNOME skin must use a compact, content-sized panel');
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_SCREENSHOT'));
    await systemScheme(systemSettings, 'dark');
    checkAppearance('gnome', 'dark');
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_GNOME_DARK_SCREENSHOT'));
    await systemScheme(systemSettings, 'light');
    checkAppearance('gnome', 'light');
    const themeContext = St.ThemeContext.get_for_stage(global.stage);
    const initialScale = themeContext.scale_factor;
    const initialWidth = extension._actor.width;
    themeContext.set_scale_factor(initialScale * 2);
    await until(() => Math.abs(extension._actor.width - initialWidth * 2) < 2,
        'Doubling the Shell scale must double the Crown HUD dimensions');
    await until(() => {
        const current = extension._actor;
        return Math.abs(area.y + area.height - current.y - current.height - 64) < 2;
    }, 'Bottom spacing must follow the Shell scale');
    check(extension._owner === owner && extension._actor === bounds,
        'Scaling must retain the actor and its presentation lease');
    check(global.display.focus_window === focused && global.stage.get_key_focus() === keyboardFocus,
        'Scaling must retain application and keyboard focus');
    for (const skin of ['logitech', 'openlogi', 'gnome']) {
        extension._settings.set_string('hud-theme', skin);
        const expectedWidth = (skin === 'logitech' ? 480 : initialWidth) * 2;
        await until(() => Math.abs(extension._actor.width - expectedWidth) < 2,
            `${skin} skin must retain its dimensions at double scale`);
        await paintFrame();
        const item = extension._items.get('volume');
        check(item.tile.height === (skin === 'logitech' ? 112 : 72) * initialScale * 2,
            `${skin} tiles must scale with their text and symbols`);
        check(Math.abs(extension._actor.x + extension._actor.width / 2 - area.x - area.width / 2) < 2,
            `${skin} HUD must remain centered after scaling`);
    }
    await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_HIDPI_SCREENSHOT'));
    themeContext.set_scale_factor(initialScale);
    await until(() => Math.abs(extension._actor.width - initialWidth) < 2,
        'Restoring the Shell scale must restore the Crown HUD dimensions');
    const actor = extension._actor;
    const foreignHide = Gio.Subprocess.new(['gdbus', 'call', '--session', '--dest', 'org.openlogi.CrownHud',
        '--object-path', '/org/openlogi/CrownHud', '--method', 'org.openlogi.CrownHud.Hide'],
    Gio.SubprocessFlags.STDOUT_SILENCE);
    await foreignHide.wait_check_async(null);
    check(extension._actor === actor, 'A foreign presenter must not dismiss the current HUD');
    const invalidShow = Gio.Subprocess.new(['gdbus', 'call', '--session', '--dest', 'org.openlogi.CrownHud',
        '--object-path', '/org/openlogi/CrownHud', '--method', 'org.openlogi.CrownHud.Show', '{}'],
    Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
    await invalidShow.wait_async(null);
    check(!invalidShow.get_successful(), 'Invalid presentation must return a D-Bus error');
    check(extension._actor === actor && extension._owner === owner,
        'Invalid presentation must leave the valid presenter and HUD intact');
    for (const feedback of [{kind: 'unknown'}, {kind: 'value', fraction: 1.01, text: 'bad'},
        {kind: 'value', fraction: 0.5, text: 'x'.repeat(33)}]) {
        const presentation = JSON.parse(JSON.stringify(extension._view));
        presentation.functions[0].feedback = feedback;
        const invalidFeedback = Gio.Subprocess.new(['gdbus', 'call', '--session', '--dest', 'org.openlogi.CrownHud',
            '--object-path', '/org/openlogi/CrownHud', '--method', 'org.openlogi.CrownHud.Show', JSON.stringify(presentation)],
        Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
        await invalidFeedback.wait_async(null);
        check(!invalidFeedback.get_successful() && extension._actor === actor && extension._owner === owner,
            'Unsupported feedback must be rejected without displacing the active presenter');
    }
    if (GLib.getenv('OPENLOGI_CROWN_TEST_AUDIO') === 'isolated') {
        extension._settings.set_string('hud-theme', 'logitech');
        await audioCommand('set-sink-volume', 'crown-hud-a', '50%');
        await checkParameter(extension, '50%', 0.5);
        await saveScreenshot(GLib.getenv('OPENLOGI_CROWN_TEST_PROGRESS_SCREENSHOT'));
        await audioCommand('set-sink-mute', 'crown-hud-a', '1');
        await checkParameter(extension, '0%', 0);
        await audioCommand('set-default-sink', 'crown-hud-b');
        await checkParameter(extension, '75%', 0.75);
        await audioCommand('set-sink-volume', 'crown-hud-b', '125%');
        await checkParameter(extension, '125%', 1);
        await audioCommand('set-sink-volume', 'crown-hud-a', '90%');
        await paintFrame();
        check(extension._items.get('volume').value.text === '125%', 'Previous output must not update the current output feedback');
    }
    extension._settings.set_string('hud-theme', 'logitech');
    advance();
    await until(() => selectedName(extension) === 'Workspaces',
        'Stable selection change must be displayed');
    const previousParameter = extension._items.get('volume');
    check(previousParameter.icon.visible && !previousParameter.value.visible && !previousParameter.fill.visible,
        'Changing function must restore the old parameter icon and clear its fill');
    check(extension._actor === bounds, 'Selection updates must reuse the visible actor without remapping');
    check(global.display.focus_window === focused, 'Selection update must retain application focus');
    check(global.stage.get_key_focus() === keyboardFocus, 'Selection update must retain keyboard focus');
    check(mapped.length === 1, 'Skin and selection updates must not remap the HUD');
    check(misplaced.length === 0, `HUD must be positioned before its first mapping: ${JSON.stringify(misplaced)}`);
    // A late enable or a quick restart must replay the current snapshot.
    extension.disable();
    await Scripting.sleep(1200);
    extension.enable();
    await until(() => selectedName(extension) === 'Workspaces' && extension._actor?.mapped && extension._actor.opacity === 255,
        'Extension re-enable must visibly recover the current selection');
    const recovered = extension._actor;
    extension.disable();
    extension.enable();
    await until(() => extension._actor !== null && extension._actor !== recovered &&
        extension._actor.mapped && extension._actor.opacity === 255,
        'A quick extension restart must recover even inside one heartbeat');
    check(global.display.focus_window === focused, 'Bridge recovery must retain application focus');
    advance();
    await until(() => extension._actor === null, 'Explicit Hide must remove the Shell actor');
    advance();
    await until(() => selectedName(extension) === 'Tabs', 'A subsequent presentation must appear');
    advance();
    await subprocess.wait_check_async(null);
    await until(() => extension._actor === null, 'Hide must remove the Shell actor');
    check(global.display.focus_window === focused, 'Dismissal must retain application focus');
    check(mapped.length === 4 && misplaced.length === 0,
        `A subsequent HUD must also map only at its final position: maps=${mapped.length}, misplaced=${JSON.stringify(misplaced)}`);
    Main.layoutManager.addChrome = addChrome;
    check(appearanceFailures.length === 0, `Skins must follow live system light/dark changes: ${appearanceFailures.join('; ')}`);
    check(layoutFailures.length === 0, `Crown layout regressions: ${layoutFailures.join('; ')}`);
    print('CROWN SHELL PASS: compact heading-free layout, centered content, authoritative volume and vertical fill, native output volume/mute/device changes, system light/dark switching, first visible position, actor reuse, three skins, selection, hide, owner lease, invalid-presentation rejection, application/keyboard focus');
    await Scripting.destroyTestWindows();
}
