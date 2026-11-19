import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
// A disposable bus with no host sockets or network. The official Screenshot
// service checks unsafe_mode for automated callers. Never ship this extension.
export default class PrivateCapture extends Extension {
    enable() {
        this.previous = global.context.unsafe_mode;
        global.context.unsafe_mode = true;
    }
    disable() {
        global.context.unsafe_mode = this.previous;
    }
}
