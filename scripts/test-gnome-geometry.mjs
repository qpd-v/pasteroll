// Synthetic contract tests: never connect to a display, clipboard or user profile.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import vm from 'node:vm';

const base = new URL('../extensions/pasteroll@pasteroll.com/', import.meta.url);
const helpers = readFileSync(new URL('geometry.js', base), 'utf8');
const {stageScaleForMonitor, ViewerTransfer} = await import(
    `data:text/javascript;base64,${Buffer.from(helpers).toString('base64')}`);

class Actor {
    constructor(options = {}) { Object.assign(this, options); this.children = []; }
    set_size(width, height) { this.width = width; this.height = height; }
    set_position(x, y) { this.x = x; this.y = y; }
    set_content(content) { this.content = content; }
    add_child(actor) { this.children.push(actor); }
    show() { this.visible = true; }
    destroy() { this.destroyed = true; for (const child of this.children) child.destroy(); }
}
class Image {
    set_data(pixels, _format, width, height, stride) {
        assert.equal(pixels.length, width * height * 4);
        assert.equal(stride, width * 4);
        Object.assign(this, {pixels, width, height});
    }
}
const context = vm.createContext({
    stageScaleForMonitor, ViewerTransfer, console,
    Clutter: {Image}, Cogl: {PixelFormat: {BGRA_8888: 1}}, St: {Widget: Actor},
    GLib: {Variant: class { constructor(signature, value) { Object.assign(this, {signature, value}); } }},
    Mtk: {Rectangle: class {}},
});
const source = readFileSync(new URL('bridge.js', base), 'utf8')
    .replace(/^import .*;\r?\n/gm, '').replace('export class Bridge', 'class Bridge');
vm.runInContext(`${source}\nglobalThis.Bridge = Bridge;`, context);
const {Bridge} = context;
function reply() {
    return {
        return_value(value) { this.value = value; },
        return_dbus_error(name) { this.error = name; },
    };
}
function bridge() {
    return Object.assign(Object.create(Bridge.prototype), {
        _authorize: () => true, _secure: () => true,
        _currentGeneration: generation => generation === 7,
        _nowMs: () => 1, _viewer: new Actor(), _viewerWidth: 3840, _viewerHeight: 2160,
        _scale: 1, _uiScale: 2, _viewerMode: true, _removeViewerFromStage() {},
    });
}

test('the old geometry protocol is rejected before peer or clipboard access', () => {
    const b = bridge();
    b._verifyPeer = () => assert.fail('an incompatible client must not attach');
    const result = reply();
    b.HandshakeAsync([2], result);
    assert.match(result.error, /Version$/);
});

test('physical and logical monitor layouts distinguish stage scale from UI scale', () => {
    for (const [width, scale] of [[3840, 1], [1920, 2]]) {
        const monitor = {x: 1920, y: -80, width, height: width * 9 / 16};
        const views = [{x: 0, y: 0, width: 1920, height: 1080, scale: 1}, {...monitor, scale}];
        assert.equal(stageScaleForMonitor(monitor, views), scale);
    }
    assert.throws(() => stageScaleForMonitor({x: 0, y: 0, width: 100, height: 100}, []));
});

test('the real companion sizes, positions and hit-tests both layouts in stage units', () => {
    for (const stageScale of [1, 2]) {
        const b = bridge();
        const monitor = {x: 1920, y: 0, width: 3840 / stageScale, height: 2160 / stageScale};
        Object.assign(b, {
            _monitorAt: () => monitor, _workFor: () => monitor, _monitorScale: () => 2,
            _stageScale: () => stageScale, _inputActive: true, _generation: 7,
            _createOverlay() { this._actor = new Actor(); },
            _addOverlayToStage() {}, _syncGrab() {},
        });
        const result = reply();
        b._showFrame(1950, 40, 1040, 880, result);
        assert.equal(result.error, undefined);
        assert.equal(result.value.value[3], 2); // UI scale, not stage conversion.
        assert.equal(b._actor.width, 1040 / stageScale);
        assert.equal(b._actor.height, 880 / stageScale);
        assert.ok(b._actor.x >= monitor.x);
        b._viewer.get_transformed_position = () => [b._actor.x, b._actor.y];
        b._viewer.get_transformed_size = () => [b._actor.width, b._actor.height];
        const point = b._viewerPoint(b._actor.x + 30, b._actor.y + 40);
        assert.deepEqual(Array.from(point), [30 * stageScale, 40 * stageScale]);
    }
});

test('4K strips preserve exact pixels and become visible only on complete commit', () => {
    const b = bridge();
    let result = reply();
    b.ViewerBeginAsync([7, 3840, 2160], result);
    const frame = result.value.value[0];
    assert.equal(b._pendingViewer.actor.visible, false);
    const maxRows = Math.floor(16 * 1024 * 1024 / (3840 * 4));
    const inputs = [];
    for (let y = 0; y < 2160; y += maxRows) {
        const height = Math.min(maxRows, 2160 - y);
        const pixels = new Uint8Array(3840 * height * 4).fill(y === 0 ? 13 : 29);
        inputs.push(pixels);
        result = reply();
        b.ViewerStripAsync([7, frame, y, height, pixels], result);
        assert.equal(result.error, undefined);
        assert.equal(b._pendingViewer.actor.visible, false);
    }
    result = reply();
    b.ViewerCommitAsync([7, frame], result);
    assert.equal(result.error, undefined);
    assert.equal(b._viewerTiles.actor.visible, true);
    assert.equal(b._pendingViewer, null);
    assert.equal(b._viewerTiles.tiles.length, 2);
    for (let i = 0; i < inputs.length; i++)
        assert.equal(b._viewerTiles.tiles[i].actor.content.pixels, inputs[i]);
    assert.equal(b._viewerTiles.tiles[1].actor.y, maxRows);
    b._scale = 2;
    b._setViewerClip();
    assert.equal(b._viewerTiles.actor.width, 1920);
    assert.equal(b._viewerTiles.tiles[1].actor.y, maxRows / 2);
});

test('incomplete, overlapping, out-of-order, oversized and stale transfers are rejected', () => {
    const cap = 16 * 1024 * 1024;
    for (const dims of [[0, 1], [4097, 1], [1, 4097]])
        assert.throws(() => new ViewerTransfer(...dims, cap));
    for (const strip of [[1, 1, 16], [0, 5, 80], [0, 1, 15], [0, 1, cap + 1]]) {
        const transfer = new ViewerTransfer(4, 4, cap);
        assert.throws(() => transfer.accept(...strip));
    }
    const b = bridge();
    let result = reply();
    b.ViewerBeginAsync([7, 4, 4], result);
    const frame = result.value.value[0];
    result = reply();
    b.ViewerCommitAsync([7, frame], result);
    assert.match(result.error, /Bounds$/);
    assert.equal(b._pendingViewer, null);
    result = reply();
    b.ViewerBeginAsync([7, 4, 4], result);
    result = reply();
    b.ViewerStripAsync([8, frame, 0, 4, new Uint8Array(64)], result);
    assert.match(result.error, /Stale$/);
    const pending = b._pendingViewer.actor;
    b._destroyViewer();
    assert.equal(pending.destroyed, true);
    assert.equal(b._pendingViewer, null);
});

test('a later small frame replaces tiled content and discards an unfinished transfer', () => {
    const b = bridge();
    const previous = new Actor();
    b._viewerTiles = {actor: previous};
    const pending = new Actor();
    b._pendingViewer = {actor: pending};
    const result = reply();
    b.ViewerPresentAsync([7, 4, 4, new Uint8Array(64)], result);
    assert.equal(result.error, undefined);
    assert.equal(previous.destroyed, true);
    assert.equal(pending.destroyed, true);
    assert.equal(b._viewerTiles, null);
    assert.equal(b._viewer.content, b._viewerImage);
});

test('complete maximum-size frames use at most four strips', () => {
    const cap = 16 * 1024 * 1024;
    const transfer = new ViewerTransfer(4096, 4096, cap);
    assert.throws(() => transfer.accept(0, 1, 4096 * 4));
    for (let y = 0; y < 4096; y += 1024)
        transfer.accept(y, 1024, cap);
    assert.equal(transfer.complete, true);
    assert.throws(() => transfer.accept(4096, 1, 4096 * 4));
});
