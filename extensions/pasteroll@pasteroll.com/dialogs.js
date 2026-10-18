import Clutter from 'gi://Clutter';
import St from 'gi://St';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const MIN_WIDTH = 420;
const MIN_HEIGHT = 260;

function fit(value, available, minimum) {
    return Math.max(Math.min(minimum, available), Math.min(value, available));
}

/**
 * A modal Shell dialog with a work-area backdrop. Normal widget input is left to
 * GNOME; the companion only intercepts clicks outside this rectangle.
 */
export function showDialog({
    work,
    title,
    width = 620,
    height = 680,
    content,
    buttons = [],
    closable = true,
    escapeCloses = true,
    onEscape = null,
    onClose = null,
}) {
    const availableWidth = Math.max(1, work.width - 32);
    const availableHeight = Math.max(1, work.height - 32);
    const dialogWidth = fit(width, availableWidth, MIN_WIDTH);
    const dialogHeight = fit(height, availableHeight, MIN_HEIGHT);

    const backdrop = new St.Widget({
        name: 'pasteroll-dialog-backdrop',
        reactive: true,
        layout_manager: new Clutter.BinLayout(),
        x: work.x,
        y: work.y,
        width: work.width,
        height: work.height,
        style_class: 'pasteroll-backdrop',
    });

    const dialog = new St.BoxLayout({
        name: 'pasteroll-dialog',
        vertical: true,
        reactive: true,
        width: dialogWidth,
        height: dialogHeight,
        x_align: Clutter.ActorAlign.CENTER,
        y_align: Clutter.ActorAlign.CENTER,
        style_class: 'pasteroll-dialog',
    });
    backdrop.add_child(dialog);

    const heading = new St.BoxLayout({style_class: 'pasteroll-dialog-heading'});
    dialog.add_child(heading);
    const titleLabel = new St.Label({
        text: title,
        x_expand: true,
        y_align: Clutter.ActorAlign.CENTER,
        style_class: 'pasteroll-dialog-title',
    });
    heading.add_child(titleLabel);

    const scroll = new St.ScrollView({
        child: content,
        x_expand: true,
        y_expand: true,
        hscrollbar_policy: St.PolicyType.NEVER,
        vscrollbar_policy: St.PolicyType.AUTOMATIC,
        style_class: 'pasteroll-dialog-scroll',
    });
    dialog.add_child(scroll);

    const footer = new St.BoxLayout({style_class: 'pasteroll-dialog-actions'});
    dialog.add_child(footer);

    let closed = false;
    let grab = null;

    function close() {
        if (closed)
            return;
        closed = true;
        try {
            if (grab)
                Main.popModal(grab);
        } catch (_) {
            // The extension may be disabled after the grab has already gone away.
        }
        grab = null;
        Main.layoutManager.removeChrome(backdrop);
        backdrop.destroy();
        try {
            onClose?.();
        } catch (_) {
            // Closing the UI must not prevent the rest of extension teardown.
        }
    }

    if (closable) {
        const closeButton = new St.Button({
            label: '×',
            can_focus: true,
            style_class: 'pasteroll-close-button',
            accessible_name: 'Close',
        });
        closeButton.connect('clicked', close);
        heading.add_child(closeButton);
    }

    for (const spec of buttons) {
        const button = new St.Button({
            label: spec.label,
            can_focus: true,
            style_class: `pasteroll-button ${spec.styleClass ?? ''}`.trim(),
        });
        button.connect('clicked', () => {
            spec.handler?.(close);
            if (spec.close !== false)
                close();
        });
        footer.add_child(button);
    }

    backdrop.connect('key-press-event', (_actor, event) => {
        if (event.get_key_symbol() !== Clutter.KEY_Escape)
            return Clutter.EVENT_PROPAGATE;
        onEscape?.(close);
        if (escapeCloses)
            close();
        return Clutter.EVENT_STOP;
    });

    Main.layoutManager.addChrome(backdrop);
    grab = Main.pushModal(backdrop, {actionMode: Shell.ActionMode.POPUP});

    return {
        actor: backdrop,
        dialog,
        content,
        close,
        get closed() {
            return closed;
        },
    };
}
