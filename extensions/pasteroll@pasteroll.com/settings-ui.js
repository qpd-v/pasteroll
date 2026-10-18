import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import St from 'gi://St';

import {showDialog} from './dialogs.js';

const MB = 1024 * 1024;
const MAX_APPS = 128;
const MAX_APP_NAME = 128;

const CLIP_STEPS = [100, 250, 500, 1000, 2500, 5000, 10000, 25000, 50000, 100000];
const TOTAL_MB_STEPS = [50, 100, 250, 500, 1024, 2048, 5120, 10240];
const CLIP_MB_STEPS = [1, 4, 16, 32, 64];
const PIN_STEPS = [0, 25, 50, 100, 200, 500, 1000];
const SIZE_STEPS = [50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200];
const AGE_STEPS = [7, 30, 90, 180, 365, 0];

function label(text, styleClass = '') {
    return new St.Label({text, style_class: styleClass});
}

function wrapLabel(text, styleClass = '') {
    const widget = new St.Label({
        text,
        x_expand: true,
        style_class: styleClass,
    });
    widget.clutter_text.line_wrap = true;
    widget.clutter_text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
    return widget;
}

function contentBox() {
    return new St.BoxLayout({
        vertical: true,
        x_expand: true,
        style_class: 'pasteroll-dialog-content',
    });
}

function section(text) {
    return label(text, 'pasteroll-section-title');
}

function row(title, description, control) {
    const box = new St.BoxLayout({style_class: 'pasteroll-setting-row'});
    const words = new St.BoxLayout({vertical: true, x_expand: true});
    words.add_child(label(title, 'pasteroll-setting-title'));
    words.add_child(wrapLabel(description, 'pasteroll-setting-description'));
    box.add_child(words);
    box.add_child(control);
    return box;
}

function orderedSteps(base, current, neverLast = false) {
    const values = [...new Set([...base, current])];
    values.sort((a, b) => {
        if (neverLast && (a === 0 || b === 0))
            return a === b ? 0 : (a === 0 ? 1 : -1);
        return a - b;
    });
    return values;
}

function choiceControl(steps, current, format, onChange) {
    const box = new St.BoxLayout({style_class: 'pasteroll-choice'});
    const less = new St.Button({
        label: '−',
        can_focus: true,
        style_class: 'pasteroll-step-button',
        accessible_name: 'Decrease',
    });
    const value = label(format(current), 'pasteroll-choice-value');
    const more = new St.Button({
        label: '+',
        can_focus: true,
        style_class: 'pasteroll-step-button',
        accessible_name: 'Increase',
    });
    box.add_child(less);
    box.add_child(value);
    box.add_child(more);

    let index = Math.max(0, steps.indexOf(current));
    const sync = () => {
        index = Math.max(0, Math.min(index, steps.length - 1));
        value.text = format(steps[index]);
        onChange(steps[index]);
    };
    less.connect('clicked', () => {
        index = Math.max(0, index - 1);
        sync();
    });
    more.connect('clicked', () => {
        index = Math.min(steps.length - 1, index + 1);
        sync();
    });
    return box;
}

function toggleControl(value, onChange) {
    const button = new St.Button({
        label: value ? 'On' : 'Off',
        can_focus: true,
        toggle_mode: true,
        checked: Boolean(value),
        style_class: `pasteroll-toggle ${value ? 'pasteroll-toggle-on' : ''}`.trim(),
    });
    button.connect('notify::checked', () => {
        const current = button.checked;
        button.label = current ? 'On' : 'Off';
        button.add_style_class_name(current ? 'pasteroll-toggle-on' : 'pasteroll-toggle-off');
        button.remove_style_class_name(current ? 'pasteroll-toggle-off' : 'pasteroll-toggle-on');
        onChange(current);
    });
    return button;
}

function megabytes(bytes) {
    return Math.max(0, Math.round(bytes / MB));
}

function formatMb(mb) {
    if (mb >= 1024 && mb % 1024 === 0)
        return `${mb / 1024} GB`;
    return `${mb} MB`;
}

function formatBytes(bytes) {
    if (bytes >= 1024 * MB && bytes % (1024 * MB) === 0)
        return `${bytes / (1024 * MB)} GB`;
    return `${Math.max(1, Math.round(bytes / MB))} MB`;
}

function formatAge(days) {
    if (days === 0)
        return 'Never';
    return days === 1 ? '1 day' : `${days} days`;
}

function validAppName(text) {
    const name = text.trim().toLowerCase();
    return name.length > 0 &&
        name.length <= MAX_APP_NAME &&
        ![...name].some(char => {
            const code = char.codePointAt(0);
            return code <= 0x1f || code === 0x7f || /[\s/\\]/.test(char);
        });
}

function appListEditor({title, description, values, onChange}) {
    const box = new St.BoxLayout({vertical: true, style: 'spacing: 7px;'});
    box.add_child(label(title, 'pasteroll-setting-title'));
    box.add_child(wrapLabel(description, 'pasteroll-setting-description'));

    const list = new St.BoxLayout({vertical: true, style: 'spacing: 4px;'});
    const entryRow = new St.BoxLayout({style_class: 'pasteroll-app-entry-row'});
    const entry = new St.Entry({
        hint_text: 'Executable name',
        x_expand: true,
        can_focus: true,
    });
    const add = new St.Button({
        label: 'Add',
        can_focus: true,
        style_class: 'pasteroll-small-button',
    });
    const status = wrapLabel('', 'pasteroll-inline-error');
    let names = [...values];

    const changed = () => {
        onChange([...names]);
    };
    const render = () => {
        list.remove_all_children();
        if (names.length === 0) {
            list.add_child(wrapLabel('No applications are excluded in this list.',
                'pasteroll-empty-list'));
            return;
        }
        for (const name of names) {
            const line = new St.BoxLayout({style_class: 'pasteroll-app-row'});
            const text = label(name, 'pasteroll-app-name');
            text.x_expand = true;
            const remove = new St.Button({
                label: 'Remove',
                can_focus: true,
                style_class: 'pasteroll-small-button',
            });
            remove.connect('clicked', () => {
                names = names.filter(value => value !== name);
                render();
                changed();
            });
            line.add_child(text);
            line.add_child(remove);
            list.add_child(line);
        }
    };
    const submit = () => {
        const name = entry.text.trim().toLowerCase();
        if (!validAppName(name)) {
            status.text = 'Use an executable name without spaces, slashes or control characters.';
            return;
        }
        if (names.includes(name)) {
            status.text = 'That application is already listed.';
            return;
        }
        if (names.length >= MAX_APPS) {
            status.text = `At most ${MAX_APPS} applications can be listed.`;
            return;
        }
        status.text = '';
        names.push(name);
        entry.text = '';
        render();
        changed();
    };
    add.connect('clicked', submit);
    entry.clutter_text.connect('activate', submit);

    render();
    entryRow.add_child(entry);
    entryRow.add_child(add);
    box.add_child(list);
    box.add_child(entryRow);
    box.add_child(status);
    return box;
}

export function showWelcome({work, onAcknowledge, onClose}) {
    const content = contentBox();
    content.add_child(wrapLabel(
        'Pasteroll keeps an encrypted history of what you copy on this computer. ' +
        'Nothing is sent anywhere.',
        'pasteroll-welcome-text'));
    content.add_child(wrapLabel(
        'On GNOME Wayland, start Pasteroll with the explicit keyboard option, then use ' +
        'Ctrl+Shift+Space. Global Ctrl+Shift+middle-click activation is unavailable ' +
        'on native GNOME applications.',
        'pasteroll-welcome-text'));
    content.add_child(wrapLabel(
        'Choose a clip with the wheel and press Enter, or select part of it first. ' +
        'The panel menu can pause recording, exclude applications and open this data folder.',
        'pasteroll-welcome-text'));
    content.add_child(wrapLabel(
        'Pasteroll does not start when you sign in unless you enable that in Settings.',
        'pasteroll-welcome-text'));

    return showDialog({
        work,
        title: 'Welcome to Pasteroll',
        width: 610,
        height: 430,
        content,
        closable: false,
        escapeCloses: false,
        buttons: [{
            label: 'I understand',
            styleClass: 'pasteroll-button-primary',
            handler: onAcknowledge,
        }],
        onClose,
    });
}

export function showSettings({work, state, onApply, onCancel, onClose}) {
    const draft = {
        paused: Boolean(state.paused),
        size: Number(state.size),
        clips: Number(state.clips),
        totalBytes: Number(state.totalBytes),
        ageDays: Number(state.ageDays),
        clipBytes: Number(state.clipBytes),
        pins: Number(state.pins),
        excludeApps: [...state.excludeApps],
        warnMultiline: Boolean(state.warnMultiline),
        terminalApps: [...state.terminalApps],
        autostart: Boolean(state.autostart),
    };
    const content = contentBox();
    content.add_child(wrapLabel(
        'Changes are checked by the application before anything is written. ' +
        'Saving restarts Pasteroll to apply the new limits.',
        'pasteroll-dialog-note'));

    content.add_child(section('Recording'));
    content.add_child(row('Pause recording',
        'New copies are not recorded while this is on.',
        toggleControl(draft.paused, value => {
            draft.paused = value;
        })));

    content.add_child(section('Startup'));
    content.add_child(row('Start when I sign in',
        'Off by default. The startup entry keeps the Wayland backend and keyboard opt-in explicit.',
        toggleControl(draft.autostart, value => {
            draft.autostart = value;
        })));

    content.add_child(section('History limits'));
    content.add_child(row('Unpinned clips',
        'The oldest unpinned clips are removed when a limit is reached.',
        choiceControl(orderedSteps(CLIP_STEPS, draft.clips), draft.clips,
            value => `${value}`, value => {
                draft.clips = value;
            })));
    content.add_child(row('Total size',
        'Encrypted history space used by unpinned clips.',
        choiceControl(orderedSteps(TOTAL_MB_STEPS, megabytes(draft.totalBytes)),
            megabytes(draft.totalBytes), formatMb, value => {
                draft.totalBytes = value * MB;
            })));
    content.add_child(row('Maximum age',
        'Older unpinned clips are removed during maintenance.',
        choiceControl(orderedSteps(AGE_STEPS, draft.ageDays, true), draft.ageDays,
            formatAge, value => {
                draft.ageDays = value;
            })));
    content.add_child(row('Largest clip',
        'A copy larger than this is not recorded.',
        choiceControl(orderedSteps(CLIP_MB_STEPS, megabytes(draft.clipBytes)),
            megabytes(draft.clipBytes), formatMb, value => {
                draft.clipBytes = value * MB;
            })));
    content.add_child(row('Pinned clips',
        'Pinned clips are kept separately from the automatic limits.',
        choiceControl(orderedSteps(PIN_STEPS, draft.pins), draft.pins,
            value => `${value}`, value => {
                draft.pins = value;
            })));

    content.add_child(section('Appearance'));
    content.add_child(row('Carousel size',
        'The carousel size relative to its normal size.',
        choiceControl(orderedSteps(SIZE_STEPS, draft.size), draft.size,
            value => `${value}%`, value => {
                draft.size = value;
            })));

    content.add_child(section('Capture exclusions'));
    content.add_child(appListEditor({
        title: 'Applications never recorded',
        description: 'Executable names such as password managers or private chat clients.',
        values: draft.excludeApps,
        onChange: values => {
            draft.excludeApps = values;
        },
    }));

    content.add_child(section('Paste safety'));
    content.add_child(row('Warn before multiline terminal paste',
        'The first Enter warns, and a second Enter pastes into a terminal.',
        toggleControl(draft.warnMultiline, value => {
            draft.warnMultiline = value;
        })));
    content.add_child(appListEditor({
        title: 'Additional terminal applications',
        description: 'Programs that should receive the multiline terminal warning.',
        values: draft.terminalApps,
        onChange: values => {
            draft.terminalApps = values;
        },
    }));

    content.add_child(wrapLabel(
        'Custom filters are added from the carousel filter menu.',
        'pasteroll-dialog-note'));

    return showDialog({
        work,
        title: 'Pasteroll settings',
        width: 700,
        height: Math.min(760, work.height - 32),
        content,
        onEscape: onCancel,
        onClose,
        buttons: [
            {label: 'Cancel', handler: onCancel},
            {
                label: 'Save changes',
                styleClass: 'pasteroll-button-primary',
                handler: () => {
                    const result = {
                        changed: true,
                        confirmed: false,
                        paused: draft.paused,
                        size: draft.size,
                        clips: draft.clips,
                        totalBytes: draft.totalBytes,
                        ageDays: draft.ageDays,
                        clipBytes: draft.clipBytes,
                        pins: draft.pins,
                        excludeApps: [...draft.excludeApps],
                        warnMultiline: draft.warnMultiline,
                        terminalApps: [...draft.terminalApps],
                        autostart: draft.autostart,
                    };
                    onApply(result);
                },
            },
        ],
    });
}

export function showConfirmation({work, clips, bytes, byAge, onConfirm, onCancel, onClose}) {
    const content = contentBox();
    content.add_child(wrapLabel(
        `Saving these settings will remove ${clips} clip(s), about ${formatBytes(bytes)}.`,
        'pasteroll-welcome-text'));
    if (byAge > 0) {
        content.add_child(wrapLabel(
            `${byAge} of those clips are outside the selected age limit.`,
            'pasteroll-welcome-text'));
    }
    content.add_child(wrapLabel(
        'This deletion cannot be undone. Choose Cancel to keep the current history.',
        'pasteroll-danger-note'));

    return showDialog({
        work,
        title: 'Remove clips and save?',
        width: 570,
        height: 360,
        content,
        onEscape: onCancel,
        onClose,
        buttons: [
            {label: 'Cancel', handler: onCancel},
            {
                label: 'Remove and save',
                styleClass: 'pasteroll-button-danger',
                handler: onConfirm,
            },
        ],
    });
}
