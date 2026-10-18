import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

/**
 * A small native GNOME panel menu. It only reports user intent; configuration and
 * process actions stay in the owning application.
 */
export function createPanel(onAction) {
    const button = new PanelMenu.Button(0.0, 'Pasteroll');
    button.add_child(new St.Icon({
        icon_name: 'edit-paste-symbolic',
        style_class: 'system-status-icon pasteroll-panel-icon',
    }));

    const activate = new PopupMenu.PopupMenuItem('Open Pasteroll');
    activate.connect('activate', () => onAction(20));
    button.menu.addMenuItem(activate);

    const pause = new PopupMenu.PopupMenuItem('Pause recording');
    pause.connect('activate', () => onAction(21));
    button.menu.addMenuItem(pause);

    const settings = new PopupMenu.PopupMenuItem('Settings…');
    settings.connect('activate', () => onAction(22));
    button.menu.addMenuItem(settings);

    const folder = new PopupMenu.PopupMenuItem('Open data folder');
    folder.connect('activate', () => onAction(23));
    button.menu.addMenuItem(folder);

    button.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
    const quit = new PopupMenu.PopupMenuItem('Quit Pasteroll');
    quit.connect('activate', () => onAction(24));
    button.menu.addMenuItem(quit);

    Main.panel.addToStatusArea('pasteroll', button);

    return {
        setPaused(paused) {
            pause.label.text = paused ? 'Resume recording' : 'Pause recording';
            button.accessible_name = paused ? 'Pasteroll, recording paused' : 'Pasteroll';
        },
        destroy() {
            if (button.menu)
                button.menu.close();
            button.destroy();
        },
    };
}
