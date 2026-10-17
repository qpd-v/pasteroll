// Development backend foundations. No arbitrary-input or test-injection API is exported.
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {Bridge} from './bridge.js';

export default class PasterollCompanion extends Extension {
    enable() {
        this._bridge = new Bridge();
    }
    disable() {
        this._bridge?.destroy();
        this._bridge = null;
    }
}
