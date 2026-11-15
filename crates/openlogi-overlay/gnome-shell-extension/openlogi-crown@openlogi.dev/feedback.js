// Read-only native feedback. Shell owns the shared mixer and volume controls;
// Crown bindings remain executable only in the agent.
import {getMixerControl} from 'resource:///org/gnome/shell/ui/status/volume.js';

export class FeedbackProvider {
    constructor(changed, control = getMixerControl()) {
        this._changed = changed;
        this._control = control;
        this._stream = null;
        this._streamSignals = [];
        this._signals = [
            control.connect('default-sink-changed', () => this._sync()),
            control.connect('state-changed', () => this._sync()),
            control.connect('stream-added', () => this._sync()),
            control.connect('stream-removed', () => this._sync()),
        ];
        this._sync();
    }

    _sync() {
        this._disconnectStream();
        this._stream = this._control.get_default_sink();
        if (this._stream) {
            this._streamSignals = ['notify::volume', 'notify::is-muted'].map(signal =>
                this._stream.connect(signal, () => this._changed()));
        }
        this._changed();
    }

    read(source) {
        if (source?.kind !== 'system_volume' || !this._stream)
            return null;
        const normal = this._control.get_vol_max_norm();
        if (normal <= 0)
            return null;
        const ratio = this._stream.is_muted ? 0 : this._stream.volume / normal;
        return {fraction: Math.min(1, Math.max(0, ratio)), text: `${Math.round(ratio * 100)}%`};
    }

    _disconnectStream() {
        for (const signal of this._streamSignals)
            this._stream.disconnect(signal);
        this._streamSignals = [];
        this._stream = null;
    }

    destroy() {
        this._disconnectStream();
        for (const signal of this._signals)
            this._control.disconnect(signal);
        this._signals = [];
    }
}
