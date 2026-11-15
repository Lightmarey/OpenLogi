import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class CrownHudPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage({title: this.gettext('Crown HUD'), icon_name: 'preferences-desktop-theme-symbolic'});
        const group = new Adw.PreferencesGroup({title: this.gettext('Appearance')});
        const skins = ['logitech', 'gnome', 'openlogi'];
        const skin = new Adw.ComboRow({title: this.gettext('Skin'),
            subtitle: this.gettext('All skins follow the system light/dark style'),
            model: Gtk.StringList.new([this.gettext('Logitech (rectangular)'), 'GNOME', 'OpenLogi'])});
        const syncSkin = () => { skin.selected = skins.indexOf(settings.get_string('hud-theme')); };
        syncSkin();
        skin.connect('notify::selected', () => settings.set_string('hud-theme', skins[skin.selected]));
        const skinSignal = settings.connect('changed::hud-theme', syncSkin);
        const margin = new Adw.SpinRow({title: this.gettext('Bottom margin'),
            subtitle: this.gettext('Logical pixels above the application monitor’s work-area edge'),
            adjustment: new Gtk.Adjustment({lower: 8, upper: 160, step_increment: 1, page_increment: 8})});
        settings.bind('bottom-margin', margin, 'value', Gio.SettingsBindFlags.DEFAULT);
        group.add(skin);
        group.add(margin);
        page.add(group);
        window.add(page);
        window.connect('close-request', () => {
            settings.disconnect(skinSignal);
            Gio.Settings.unbind(margin, 'value');
            return false;
        });
    }
}
