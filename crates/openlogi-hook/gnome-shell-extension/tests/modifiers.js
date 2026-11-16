import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

Gio._promisify(Gio.Subprocess.prototype, 'communicate_utf8_async');

function check(condition, message) {
    if (!condition)
        throw new Error(message);
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

async function expectModifiers(expected) {
    await paintFrame();
    const process = Gio.Subprocess.new([
        'gdbus', 'call', '--session', '--dest', 'org.openlogi.Frontmost',
        '--object-path', '/org/openlogi/Frontmost', '--method',
        'org.openlogi.Frontmost.GetHeldModifiers',
    ], Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
    const [output, error] = await process.communicate_utf8_async(null, null);
    check(process.get_successful(), `Native modifier query failed: ${error}`);
    check(output.trim() === `(${expected.join(', ')})`,
        `Expected held modifiers ${expected}, received ${output.trim()}`);
}

export const METRICS = {};

export async function run() {
    await Main.extensionManager._initializationPromise;
    const extension = Main.extensionManager.lookup('openlogi-frontmost@openlogi.dev')?.stateObj;
    check(extension, 'The desktop bridge must be enabled');
    const keyboard = global.stage.get_context().get_backend().get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
    const held = new Set();
    const key = (symbol, pressed) => {
        if (pressed)
            held.add(symbol);
        else
            held.delete(symbol);
        keyboard.notify_keyval(GLib.get_monotonic_time(), symbol,
            pressed ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
    };
    try {
        await expectModifiers([false, false, false, false]);
        key(Clutter.KEY_Shift_L, true);
        await expectModifiers([true, false, false, false]);
        key(Clutter.KEY_Shift_R, true);
        key(Clutter.KEY_Shift_L, false);
        await expectModifiers([true, false, false, false]);
        key(Clutter.KEY_Shift_R, false);
        await expectModifiers([false, false, false, false]);
        key(Clutter.KEY_Control_L, true);
        key(Clutter.KEY_Alt_L, true);
        key(Clutter.KEY_Super_L, true);
        await expectModifiers([false, true, true, true]);
    } finally {
        for (const symbol of held)
            key(symbol, false);
    }
    await expectModifiers([false, false, false, false]);
    print('CROWN MODIFIERS PASS: native Wayland state, both Shift keys, Control/Alt/Super and balanced release over D-Bus');
}
