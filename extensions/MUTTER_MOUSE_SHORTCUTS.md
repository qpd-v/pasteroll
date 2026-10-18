# Native GNOME mouse activation: long-term implementation plan

Superseded: the user rejected compositor changes. The selected implementation is
the Pasteroll-owned host helper described in [INPUT_HELPER.md](INPUT_HELPER.md).
The remainder is historical research, not the active implementation plan.

Status: design checkpoint, 2026-10-17. No compositor patch is implemented or
tested by this document. Current Pasteroll GNOME mouse activation remains
unavailable; its keyboard mode remains explicit opt-in.

## Intended result

Ctrl+Shift+middle-click opens Pasteroll over native Wayland applications, using
the same physical gesture as Windows and X11. The application underneath receives
neither the activation button press nor its release. Ordinary middle-click,
primary-selection paste, scrolling and dragging retain their normal behavior.

Implement a generic modifier-plus-pointer-button shortcut facility in Mutter,
integrate its policy and lifecycle with GNOME Shell, and consume it through the
existing Pasteroll companion. Pursue upstream inclusion so users can eventually
get support through normal desktop updates. A Pasteroll installer cannot make
this facility appear in an unmodified older compositor.

This is a proposal, not an existing GNOME API. API names and signatures must be
settled against source, introspection requirements and upstream review. No
application-specific Pasteroll behavior belongs in Mutter.

## What the investigation established

- Mutter routes native input before a normal Shell stage event handler can
  intercept it. Its compositor event path is the place to investigate matching
  and claiming a button sequence, while preserving seat and pointer bookkeeping.
  [Pinned event pipeline][events] and [pointer implementation][pointer].
- GNOME maintainers previously recommended a Mutter mouse-shortcut API for this
  class of request. That discussion is historical design guidance, not proof of
  support in every current release. [Maintainer discussion][discussion].
- The current XDG shortcut specification describes keyboard triggers. A mouse
  trigger cannot simply be passed as an undocumented string to GlobalShortcuts.
  A standards extension and desktop backend support are separate work.
  [Shortcut specification][shortcuts] and [portal interface][portal].
- The local runtime is GNOME Shell 46.0 with Mutter 46.2. The audited Mutter
  development source declares 51.1 and API 51, with newer build dependencies.
  Treat current upstream work and a GNOME 46 test backport as different targets.
  [Pinned build requirements][build].

Pinned investigation revisions:

| Project | Revision |
| --- | --- |
| Mutter | `ea9a191ace6303f0b56689ae1905bda3512d8c86` |
| GNOME Shell | `0dea3091e0ad20ef18bdcd3e42545870c2f05983` |
| xdg-desktop-portal | `8eba842f30070e2d90e16832462ad8171bd5beed` |

Recheck these before implementation. A pinned development revision does not
establish released support or promise compatibility with an installed desktop.

## 1. Mutter: registration, dispatch and button ownership

The draft registration contract needs a logical pointer button, meaningful
keyboard modifiers, a registration identity, a policy decision and an activation
notification. It must report conflicts and unsupported cases explicitly. Start
with one button plus modifiers; additional gestures are separate extensions.

Define button numbering at the API boundary rather than mixing Clutter logical
button numbers with Linux event codes. Match modifiers from the compositor seat
state, including keyboards separate from the mouse. Normalize lock modifiers
without accidentally accepting extra Alt or Super modifiers. Decide and document
left/right modifier and accessibility-latched modifier semantics.

Keep the ownership of each physical button sequence inside the compositor:

| Sequence state | Required behavior |
| --- | --- |
| Eligible chord press | Claim before client delivery; issue one activation |
| Ordinary press already delivered | Preserve its matching release; never claim retrospectively |
| Claimed button still held | Retain ownership even if modifiers are released |
| Matching claimed release | Consume it and complete internal bookkeeping |
| Registration removed while held | Cancel future activation; drain the owned sequence safely |
| Device removed or seat reset | Retire state using device lifetime, not a reused numeric ID |

A timeout must not turn an already claimed press into an orphan release sent to
a client. Keep a bounded entry per active device/button until release or a real
device/seat reset. Re-registration cannot inherit activation rights from the old
owner. Reentrant callbacks, duplicate presses and synchronous owner destruction
must not cause a second activation or invalidate objects in use.

Audit both physical seat state and client-visible pressed state. A shortcut must
not create a client implicit grab, give that client an activation click serial,
or disrupt an unrelated existing drag. Claiming an event is not equivalent to
returning early from every stage of pointer processing. Preserve focus updates,
accessibility state, cursor behavior and valid client event pairs.

Define dispatch priority relative to window-manager mouse bindings, modal grabs,
input capture and shortcut inhibition. Resolve conflicts explicitly; do not
silently steal a reserved binding. Evaluate new activations only in allowed
contexts, but continue safe cleanup of previously claimed sequences during a
policy transition. Audit XWayland delivery independently.

Expose semantic shortcut activation, not an unrestricted stream of mouse or
keyboard events. Trusted accessibility and remote-input paths need an explicit
policy; rejecting every virtual device would also break legitimate input and
the isolated test driver.

## 2. GNOME Shell: policy, settings and ownership

Add the matching Shell integration and GObject introspection coverage. Reuse
existing action-mode policy concepts where appropriate; do not assume the
keyboard binding implementation already handles pointer buttons.

- Registration lifetime follows its owner; extension disable releases it.
- Lock screen, greeter, overview, modal dialogs and desktop captures have defined
  activation policy. Pasteroll initially requests normal unlocked desktop mode.
- A conflict leaves mouse capability unavailable with an actionable diagnostic.
- User settings can disable or rebind the gesture. Keep the chosen binding stable
  across restart, with no automatic takeover of another application's shortcut.
- Registration failure and companion/app crashes leave normal input usable.
- No private clipboard contents, typed keys or raw input histories enter logs.

Port the companion to each supported Shell API and test it. Updating extension
metadata alone does not establish compatibility with newer GNOME releases.

## 3. Pasteroll: capability and activation context

Adapt these existing integration points after the compositor path works:

| Component | Work |
| --- | --- |
| `extensions/pasteroll@pasteroll.com/extension.js` | Own native registration and tear it down on disable |
| `extensions/pasteroll@pasteroll.com/bridge.js` | Advertise mouse support only after registration succeeds; preserve authenticated peer and focus checks |
| `crates/pasteroll-linux/src/wayland/protocol.rs` | Specify a versioned activation contract and compatibility behavior |
| `crates/pasteroll-linux/src/wayland/session.rs` | Select native mouse mode only when genuinely available |
| `crates/pasteroll-linux/src/wayland/run.rs` | Route the native activation into the existing controller |

Bridge2 currently requires an explicit keyboard opt-in for `Show`. Do not fake
that flag for mouse activation or silently reinterpret its meaning. Introduce a
compatible, versioned path with a short-lived, one-use activation authorization
bound to the authenticated peer, compositor event and intended target.

Capture the target identity, pointer position, monitor/scale and event time at
activation. Revalidate before showing or pasting if asynchronous work could have
changed focus. Keep activation authorization separate from paste authorization:
opening the carousel must not authorize an arbitrary later paste or click.
Consume the authorization once; reject stale, replayed or wrong-owner requests.

The opening middle-click must never become a carousel selection/paste click.
Preserve current generation, owner-loss, watchdog, privilege and sensitive-source
checks. The keyboard mode remains a deliberately selected compatibility option.
Autostart must preserve the selected mode and explain lost capability after a
desktop update, rather than silently changing the shortcut.

## 4. Desktop standard and portal integration

After the compositor proof, prepare a separate proposal for pointer-button
triggers in the XDG shortcut specification and GlobalShortcuts portal stack.
Specify trigger encoding, consent, conflicts, revocation, activation/deactivation
semantics and device/modifier behavior. Include GNOME backend and settings UI
work; consider how another compositor could implement the same contract.

A portal client should receive a semantic action after user authorization,
without access to unrelated input. Pasteroll can adopt it when the required
backend capability actually ships. Flatpak packaging alone supplies none of
these missing compositor or portal features. Clipboard capture, overlays and
paste integration still need their own supported mechanisms.

Upstream agreement is an external dependency. Local code and tests cannot promise
acceptance, a release date, distro backports or removal of the GNOME companion.
Prepare local review material first; posting issues or merge requests is a
separate external action.

## Implementation milestones and exit criteria

### A. Isolated environment and baseline

Build a matched upstream Mutter/Shell pair in a private environment and prefix.
Record source revisions, dependency versions, configuration and hashes. Keep
pristine and modified trees separate and retain a reproducible patch series.
Do not replace the system compositor or mix it with a mismatched Shell.

Run a native client recording received button pairs. Demonstrate the current
unpatched chord behavior and the existing keyboard Pasteroll path. These are
baseline observations, not new mouse-support successes.

### B. Compositor patch and upstream tests

Implement the registry, sequence ownership and semantic notification. Add Mutter
tests for registration, conflicts, dispatch order, release pairing and owner/
device lifetime. Run the supported non-KVM test suite and relevant sanitizer
build. Verify introspection and a matched Shell build against staged Mutter.

### C. Shell and real Pasteroll integration

Implement normal-mode registration and the versioned Pasteroll activation path.
Show the real carousel, scroll, select and paste into native clients using the
original chord. Measure client events to prove the activation click was consumed.
Repeat ordinary clicks to demonstrate normal delivery. Run app regression suites
on Windows, Linux X11 and native GNOME.

### D. Compatibility and failure validation

Cover at least the following matrix; mark unsupported or untested cells honestly:

- Native GTK, native Qt, a real terminal and XWayland clients.
- Ordinary middle-click/primary selection, double-click, wheel and drag.
- Modifiers released before the button; modifiers pressed after an ordinary
  button press; extra modifiers; lock modifiers; sticky keys.
- Two mice, separate keyboards, duplicate events, unplug/replug and device reuse.
- Extension disable, app/companion failure, unregister/re-register and compositor
  policy transitions while the activation button is held.
- Lock/unlock, modal dialogs, overview, shortcut inhibition and input capture.
- Multiple monitors, mixed scale, negative coordinates and rapid focus changes.
- Physical devices/GPU and a real login session, in addition to headless tests.

### E. Upstream review and distribution

Prepare a generic rationale, minimal reproducer, independently reviewable patch
series, API documentation and test results. Rebase onto agreed upstream targets.
Keep any GNOME 46 proof/backport explicitly separate from the forward-looking
patch. Maintain only a narrow, temporary tested backport if necessary.

Publish Pasteroll support only for the actual tested desktop versions and
capabilities. An experimental patched-compositor build must be labeled as such;
stock GNOME mouse support requires the relevant upstream/distro implementation.
Portal standardization can continue separately once the GNOME path is proven.

Each milestone ends with its diff, checks and remaining gaps before starting
the next phase. No upstream posting or release publication is implied.

## Local environment constraints

Read-only checks found Docker running and an existing GNOME 46 headless test
environment. Meson, Ninja and GLib/Mutter development headers are absent in the
current Ubuntu WSL environment. Its runtime packages do not satisfy the audited
upstream build requirements. Provision a separate modern development environment.

The Windows C: volume has about 4.5 GB free; D: has about 155 GiB free at this
checkpoint. WSL reports a large virtual filesystem, which does not prove the
underlying Windows volume has equivalent free space. Verify where container/
distribution storage physically resides before downloading an SDK or compiling.
A private build environment on D: is a candidate; it has not been provisioned.
Do not move the user's existing Docker/WSL storage or delete their files.

The present environment has no `/dev/uinput` or `/dev/kvm`. Neither is necessary
for the initial compositor experiment: existing isolated tests use compositor
virtual devices, and upstream separates non-KVM tests. Hardware and real-session
validation remain later explicit gates.

## Evidence boundary

Existing Phase 3 results under `target-dev/wayland-phase3/` validate the keyboard
backend and existing GNOME interaction features. They do not validate this plan's
mouse API. Keep new compositor results in a separate artifact directory, with
unpatched/patched source identities and exact test inputs.

No compositor code, app runtime code, installed desktop, release artifact or
website was changed for this design checkpoint.

[events]: https://github.com/GNOME/mutter/blob/ea9a191ace6303f0b56689ae1905bda3512d8c86/src/core/events.c
[pointer]: https://github.com/GNOME/mutter/blob/ea9a191ace6303f0b56689ae1905bda3512d8c86/src/wayland/meta-wayland-pointer.c
[build]: https://github.com/GNOME/mutter/blob/ea9a191ace6303f0b56689ae1905bda3512d8c86/meson.build
[discussion]: https://discourse.gnome.org/t/how-to-bind-modifier-mousebutton-in-gjs/3743
[shortcuts]: https://specifications.freedesktop.org/shortcuts/latest/
[portal]: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
