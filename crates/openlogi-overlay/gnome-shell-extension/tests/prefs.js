import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk?version=4.0';

Gio.resources_register(Gio.Resource.load(GLib.getenv('OPENLOGI_GNOME_EXTENSIONS_RESOURCE')));
const directory = GLib.getenv('OPENLOGI_CROWN_TEST_EXTENSION_DIR');
const {default: Preferences} = await import(`file://${directory}/prefs.js`);
const [, metadataBytes] = GLib.file_get_contents(`${directory}/metadata.json`);
const metadata = JSON.parse(new TextDecoder().decode(metadataBytes));
const preferences = new Preferences({...metadata, path: directory, dir: Gio.File.new_for_path(directory)});
Gtk.init();
Adw.init();
const window = new Adw.PreferencesWindow();
preferences.fillPreferencesWindow(window);

function check(condition, message) {
    if (!condition)
        throw new Error(message);
}

function find(widget, type) {
    if (widget instanceof type)
        return widget;
    for (let child = widget.get_first_child(); child; child = child.get_next_sibling()) {
        const result = find(child, type);
        if (result)
            return result;
    }
    return null;
}

const settings = preferences.getSettings();
const skin = find(window, Adw.ComboRow);
const margin = find(window, Adw.SpinRow);
check(skin && margin, 'Preferences must expose native skin and margin controls');
check(skin.selected === 0 && margin.value === 32, 'Preferences must display rectangular defaults');
skin.selected = 1;
check(settings.get_string('hud-theme') === 'gnome', 'Choosing a skin must persist it');
settings.set_string('hud-theme', 'openlogi');
check(skin.selected === 2, 'External skin changes must update the chooser');
margin.value = 64;
check(settings.get_int('bottom-margin') === 64, 'Changing the margin must persist it');
settings.set_int('bottom-margin', 32);
check(margin.value === 32, 'External margin changes must update the control');
if (GLib.getenv('LANGUAGE') === 'zh_CN')
    check(skin.title === '皮肤', 'The native preferences must load their Chinese catalog');
print('CROWN PREFERENCES PASS: native controls, defaults, bidirectional skin/margin settings, localization');
window.close();
