import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {FeedbackProvider} from './feedback.js';

const INTERFACE = `<node><interface name="org.openlogi.CrownHud">
<method name="GetVersion"><arg direction="out" type="u"/></method>
<method name="Show"><arg direction="in" type="s"/></method>
<method name="Hide"/>
</interface></node>`;
const SERVICE = 'org.openlogi.CrownHud';
// GNOME 48 added orientation; GNOME 51 removed the older vertical property.
const VERTICAL_LAYOUT = 'orientation' in St.BoxLayout.prototype
    ? {orientation: Clutter.Orientation.VERTICAL} : {vertical: true};

function validate(payload) {
    if (payload.length > 65536)
        throw new Error('Crown presentation is too large');
    const view = JSON.parse(payload);
    if (!Array.isArray(view.functions) || view.functions.length < 1 || view.functions.length > 8)
        throw new Error('Invalid Crown function count');
    const ids = new Set();
    for (const entry of view.functions) {
        if (typeof entry.id !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(entry.id) || ids.has(entry.id) ||
            typeof entry.name !== 'string' || [...entry.name].length < 1 || [...entry.name].length > 64 ||
            typeof entry.svg !== 'string' || entry.svg.length > 16384)
            throw new Error('Invalid Crown function presentation');
        ids.add(entry.id);
        if (entry.feedback !== undefined && entry.feedback?.kind !== 'system_volume')
            throw new Error('Invalid Crown parameter feedback');
    }
    if (!ids.has(view.selected))
        throw new Error('Unknown selected Crown function');
    if (typeof view.accent !== 'string' || !/^rgba\([\d.]+,[\d.]+,[\d.]+,[\d.]+\)$/.test(view.accent))
        throw new Error('Invalid Crown accent');
    return view;
}

function label(text) {
    const actor = new St.Label({text, style_class: 'openlogi-crown-name',
        x_align: Clutter.ActorAlign.CENTER, x_expand: true, reactive: false, can_focus: false});
    actor.clutter_text.ellipsize = Pango.EllipsizeMode.END;
    actor.clutter_text.set_single_line_mode(true);
    return actor;
}

function cssColor(color) {
    return `rgba(${color.red},${color.green},${color.blue},${color.alpha / 255})`;
}

export default class CrownHudExtension extends Extension {
    enable() {
        this._actor = null;
        this._owner = null;
        this._ownerWatch = 0;
        this._positionLater = 0;
        this._view = null;
        this._items = new Map();
        this._feedback = new FeedbackProvider(() => this._updateValues());
        this._settings = this.getSettings();
        this._signals = [
            [this._settings, this._settings.connect('changed', () => this._refresh())],
            [St.ThemeContext.get_for_stage(global.stage),
                St.ThemeContext.get_for_stage(global.stage).connect('changed', () => this._refresh())],
            [Main.layoutManager, Main.layoutManager.connect('monitors-changed', () => this._queuePosition())],
            [global.display, global.display.connect('workareas-changed', () => this._queuePosition())],
            [global.display, global.display.connect('notify::focus-window', () => this._queuePosition())],
        ];
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, '/org/openlogi/CrownHud');
        this._bus = Gio.bus_own_name(Gio.BusType.SESSION, SERVICE, Gio.BusNameOwnerFlags.NONE, null, null, null);
    }

    disable() {
        for (const [object, id] of this._signals ?? [])
            object.disconnect(id);
        this._signals = [];
        this._hide();
        this._feedback?.destroy();
        this._feedback = null;
        this._settings = null;
        this._dbus?.unexport();
        this._dbus = null;
        if (this._bus)
            Gio.bus_unown_name(this._bus);
        this._bus = 0;
    }

    GetVersion() { return 1; }

    ShowAsync([payload], invocation) {
        let view;
        try {
            view = validate(payload);
        } catch (error) {
            invocation.return_dbus_error('org.openlogi.CrownHud.InvalidPresentation', error.message);
            return;
        }
        try {
            const owner = invocation.get_sender();
            if (owner !== this._owner) {
                this._hide();
                this._owner = owner;
                this._ownerWatch = Gio.bus_watch_name(Gio.BusType.SESSION, owner, Gio.BusNameWatcherFlags.NONE,
                    null, () => { if (this._owner === owner) this._hide(); });
            }
            this._show(view);
            invocation.return_value(null);
        } catch (error) {
            this._hide();
            console.error('Crown HUD presentation failed', error);
            invocation.return_dbus_error('org.openlogi.CrownHud.InvalidPresentation', error.message);
        }
    }

    HideAsync(_parameters, invocation) {
        if (invocation.get_sender() === this._owner)
            this._hide();
        invocation.return_value(null);
    }

    _show(view) {
        const previous = this._view;
        this._view = view;
        if (!this._actor) {
            // A mapped actor at (0, 0) can be painted before an idle callback.
            // Resolve its mapped theme while transparent, then position and reveal
            // in the same before-redraw callback; never paint a placeholder frame.
            this._actor = new St.BoxLayout({...VERTICAL_LAYOUT, visible: false, opacity: 0,
                reactive: false, can_focus: false, style_class: 'openlogi-crown-hud'});
            Main.layoutManager.addChrome(this._actor, {trackFullscreen: false});
        }
        if (!previous || previous.functions.length !== view.functions.length ||
            previous.functions.some((entry, index) => entry.id !== view.functions[index].id)) {
            // Profile structure changes replace only children; ordinary selection
            // and skin updates retain the parent and each icon actor.
            for (const child of this._actor.get_children())
                child.destroy();
            this._items.clear();
            for (let offset = 0; offset < view.functions.length; offset += 4) {
                const row = new St.BoxLayout({style_class: 'openlogi-crown-row',
                    x_align: Clutter.ActorAlign.CENTER, reactive: false, can_focus: false});
                this._actor.add_child(row);
                for (const entry of view.functions.slice(offset, offset + 4)) {
                    const tile = new St.Widget({layout_manager: new Clutter.BinLayout(),
                        style_class: 'openlogi-crown-tile', clip_to_allocation: true,
                        reactive: false, can_focus: false});
                    const fill = new St.Widget({style_class: 'openlogi-crown-fill',
                        x_align: Clutter.ActorAlign.START, y_align: Clutter.ActorAlign.FILL,
                        x_expand: true, y_expand: true, reactive: false, can_focus: false, visible: false});
                    const content = new St.BoxLayout({...VERTICAL_LAYOUT, style_class: 'openlogi-crown-content',
                        x_align: Clutter.ActorAlign.CENTER, y_align: Clutter.ActorAlign.CENTER,
                        reactive: false, can_focus: false});
                    const symbol = new St.Widget({layout_manager: new Clutter.BinLayout(),
                        x_align: Clutter.ActorAlign.CENTER, style_class: 'openlogi-crown-symbol',
                        reactive: false, can_focus: false});
                    const icon = new St.Icon({x_align: Clutter.ActorAlign.CENTER,
                        reactive: false, can_focus: false});
                    const name = label(entry.name);
                    const value = label('');
                    value.style_class = 'openlogi-crown-value';
                    value.y_align = Clutter.ActorAlign.CENTER;
                    value.visible = false;
                    symbol.add_child(icon);
                    symbol.add_child(value);
                    row.add_child(tile);
                    tile.add_child(fill);
                    tile.add_child(content);
                    content.add_child(symbol);
                    content.add_child(name);
                    tile.connect('notify::allocation', () => {
                        const item = this._items.get(entry.id);
                        if (item)
                            item.fill.width = tile.width * item.fraction;
                    });
                    this._items.set(entry.id, {tile, fill, content, symbol, icon, value, name,
                        svg: null, fraction: 0});
                }
            }
        }
        this._updateAppearance();
        this._queuePosition();
    }

    _updateAppearance() {
        const view = this._view;
        const classic = this._settings.get_string('hud-theme') === 'logitech';
        const native = this._settings.get_string('hud-theme') === 'gnome';
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const columns = Math.min(4, view.functions.length);
        const tileWidth = (classic ? 160 : 84) * scale;
        this._actor.width = tileWidth * columns + (classic ? 0 : 4 * (columns - 1) + 24) * scale;
        // Shell OSDs deliberately stay dark in the light variant. Use its
        // adaptive panel and button styles for this non-interactive HUD.
        this._actor.style_class = `openlogi-crown-hud${native ? ' popup-menu-content gnome' : ''}${classic ? ' logitech' : ''}${!native && !classic ? ' openlogi' : ''}`;
        this._actor.style = null;
        const foreground = cssColor(this._actor.get_theme_node().get_foreground_color());
        for (const entry of view.functions) {
            const item = this._items.get(entry.id);
            const {tile, content, symbol, icon, name, value, fill} = item;
            const active = entry.id === view.selected;
            tile.width = tileWidth;
            tile.height = (classic ? 112 : 72) * scale;
            content.width = (classic ? 128 : 84) * scale;
            fill.y_align = classic ? Clutter.ActorAlign.FILL : Clutter.ActorAlign.END;
            fill.height = classic ? -1 : 3 * scale;
            if (active)
                tile.add_style_pseudo_class('checked');
            else
                tile.remove_style_pseudo_class('checked');
            // StIcon applies the theme scale to icon_size itself.
            icon.icon_size = classic ? 32 : 24;
            symbol.width = (classic ? 112 : 44) * scale;
            symbol.height = (classic ? 32 : 44) * scale;
            symbol.style_class = `${native ? `button ${active ? 'default ' : ''}` : ''}openlogi-crown-symbol`;
            symbol.style = !native && !classic && active ? `background-color: ${view.accent};` : null;
            icon.style_class = 'openlogi-crown-icon';
            if (active)
                icon.add_style_pseudo_class('checked');
            else
                icon.remove_style_pseudo_class('checked');
            const glyph = cssColor(icon.get_theme_node().get_foreground_color());
            const svg = entry.svg.replaceAll('currentColor', glyph);
            if (item.svg !== svg) {
                icon.gicon = new Gio.BytesIcon({bytes: new GLib.Bytes(new TextEncoder().encode(svg))});
                item.svg = svg;
            }
            name.text = entry.name;
            name.style = `color: ${foreground};`;
            name.height = -1;
            name.clutter_text.line_wrap = classic;
            name.clutter_text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
            value.style = `color: ${glyph};`;
            fill.style = classic || native ? null : `background-color: ${view.accent};`;
        }
        this._updateValues();
    }

    _updateValues() {
        if (!this._view || !this._feedback)
            return;
        for (const entry of this._view.functions) {
            const item = this._items.get(entry.id);
            if (!item)
                continue;
            const feedback = entry.id === this._view.selected ? this._feedback.read(entry.feedback) : null;
            item.fraction = feedback?.fraction ?? 0;
            item.fill.width = item.tile.width * item.fraction;
            item.fill.visible = feedback !== null;
            item.value.visible = feedback !== null;
            item.icon.visible = feedback === null;
            item.value.text = feedback?.text ?? '';
            if (feedback)
                item.tile.add_style_pseudo_class('parameter');
            else
                item.tile.remove_style_pseudo_class('parameter');
        }
    }

    _refresh() {
        if (!this._view)
            return;
        try {
            this._updateAppearance();
            this._queuePosition();
        } catch (error) {
            this._hide();
            console.error('Crown HUD theme update failed', error);
        }
    }

    _queuePosition() {
        if (!this._actor || this._positionLater)
            return;
        this._positionLater = global.compositor.get_laters().add(Meta.LaterType.BEFORE_REDRAW, () => {
            this._positionLater = 0;
            const monitor = global.display.focus_window?.get_monitor() ?? Main.layoutManager.primaryIndex;
            if (monitor < 0)
                return GLib.SOURCE_REMOVE;
            const area = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(monitor);
            // Shell theme spacing can change on mapping. Measure after mapping,
            // while the new actor is still transparent, before the first paint.
            this._actor.show();
            const width = this._actor.width;
            const [, height] = this._actor.get_preferred_height(width);
            const margin = this._settings.get_int('bottom-margin') *
                St.ThemeContext.get_for_stage(global.stage).scale_factor;
            this._actor.set_position(area.x + Math.max(0, (area.width - width) / 2),
                area.y + Math.max(0, area.height - height - margin));
            this._actor.opacity = 255;
            return GLib.SOURCE_REMOVE;
        });
    }

    _clearActor() {
        if (this._positionLater)
            global.compositor.get_laters().remove(this._positionLater);
        this._positionLater = 0;
        this._actor?.destroy();
        this._actor = null;
        this._items.clear();
    }

    _hide() {
        this._clearActor();
        this._view = null;
        if (this._ownerWatch)
            Gio.bus_unwatch_name(this._ownerWatch);
        this._ownerWatch = 0;
        this._owner = null;
    }
}
