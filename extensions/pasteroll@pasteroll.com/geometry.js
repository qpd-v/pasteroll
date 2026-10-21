// GNOME has two coordinate layouts. In physical layout, a 200% monitor still
// uses physical stage coordinates; in logical layout its stage view scales by 2.
// Display/UI scale and stage-to-device scale must therefore stay independent.
export function stageScaleForMonitor(monitor, views) {
    const x = monitor.x + monitor.width / 2;
    const y = monitor.y + monitor.height / 2;
    const view = views.find(v => x >= v.x && y >= v.y &&
        x < v.x + v.width && y < v.y + v.height);
    if (!view || !Number.isFinite(view.scale) || view.scale <= 0 || view.scale > 64)
        throw new Error('monitor stage view is unavailable');
    return view.scale;
}

// Each message stays within the 16 MiB transport cap. A complete viewer can be
// at most 4096x4096 (64 MiB); consecutive full-width strips prevent overlap,
// gaps and unbounded accumulation. Pixels are never resampled.
export class ViewerTransfer {
    constructor(width, height, maxBytes) {
        if (!Number.isInteger(width) || !Number.isInteger(height) ||
            width < 1 || height < 1 || width > 4096 || height > 4096)
            throw new Error('invalid viewer dimensions');
        this.width = width;
        this.height = height;
        this.maxBytes = maxBytes;
        this.nextY = 0;
    }

    accept(y, height, bytes) {
        if (!Number.isInteger(y) || !Number.isInteger(height) ||
            y !== this.nextY || height < 1 || y + height > this.height ||
            bytes !== this.width * height * 4 || bytes > this.maxBytes ||
            height !== Math.min(Math.floor(this.maxBytes / (this.width * 4)), this.height - y))
            throw new Error('invalid viewer strip');
        this.nextY += height;
    }

    get complete() {
        return this.nextY === this.height;
    }
}
