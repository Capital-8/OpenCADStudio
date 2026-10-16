//! Custom-grip drag lifecycle (API v11): the plugin-owned twin of the built-in
//! grip drag in `update/viewport.rs`.
//!
//! Same ride as built-ins — Esc cancels, mouse-up settles, another command
//! starting mid-drag cancels, tab switches cancel — but the host applies
//! NOTHING. Move events stream to the owning plugin (coalesced: at most one
//! in flight, superseded points dropped), the plugin's preview wires are the
//! only live feedback (no host ghost), and the settle is applied by the
//! plugin inside its own `push_undo` scope: the settled drag totals exactly
//! the plugin's one labeled undo step. Cancel writes nothing.

use std::time::Duration;

use glam::DVec3;
use iced::{Point, Rectangle, Task};

use crate::app::{Message, OpenCADStudio};
use crate::plugin::grip_points::{self, ActivePluginGrip, GripCandidate};

/// Poll slice for an outstanding coalesced move (kept tiny: the pump
/// reschedules itself, so the app loop never blocks on plugin latency).
const MOVE_POLL_SLICE: Duration = Duration::from_millis(8);
/// How long the settle waits for an outstanding move's answer before
/// proceeding (the CursorMove floor: a plugin slower than this is treated as
/// unresponsive for the drag, and the settle is still delivered).
const MOVE_DRAIN_BUDGET: Duration = Duration::from_millis(500);
/// Self-rescheduling pump cadence while a drag is live.
const PUMP_CADENCE: Duration = Duration::from_millis(10);

/// A future that resolves after the pump cadence (the same thread+oneshot
/// shape the dock hover delay uses — `iced::time::sleep` is not available in
/// this iced version).
fn grip_pump_delay() -> impl std::future::Future<Output = ()> {
    #[cfg(not(target_arch = "wasm32"))]
    let rx = {
        let (tx, rx) = iced::futures::channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            std::thread::sleep(PUMP_CADENCE);
            let _ = tx.send(());
        });
        rx
    };
    #[cfg(target_arch = "wasm32")]
    let rx = {
        // No custom grips exist on wasm (no plugin processes); a resolved
        // future keeps the type uniform.
        let (_tx, rx) = iced::futures::channel::oneshot::channel::<()>();
        rx
    };
    async move {
        let _ = rx.await;
    }
}

impl OpenCADStudio {
    /// Custom grips of tab `i` that are visible in the current view, with
    /// their tile-relative screen positions. Hover, press and render share
    /// this one query — it goes through the per-tab uniform grid with
    /// viewport culling, never a linear scan of every declared grip.
    pub(in crate::app) fn plugin_grips_in_view(&self, i: usize) -> Vec<(GripCandidate, Point)> {
        if self.tabs.len() <= i || self.tabs[i].plugin_grips.is_empty() {
            return Vec::new();
        }
        let (vw, vh) = self.tabs[i].scene.selection.borrow().view.vp_size;
        let edit_frame = self.tabs[i].scene.viewport_edit_frame((vw, vh));
        let tile = match &edit_frame {
            Some((_, full)) => *full,
            None => self.tabs[i].scene.active_model_tile_bounds(vw, vh),
        };
        let bounds = Rectangle {
            width: tile.width,
            height: tile.height,
            ..Rectangle::default()
        };
        let cam = edit_frame.map_or_else(
            || self.tabs[i].scene.camera.borrow().clone(),
            |(cam, _)| cam,
        );
        // World rectangle of the view: unproject the tile corners on the
        // camera's target plane (the same plane cursor picks fall back to).
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(bounds.width, 0.0),
            Point::new(0.0, bounds.height),
            Point::new(bounds.width, bounds.height),
        ];
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let mut any = false;
        for corner in corners {
            let world = cam.pick_on_target_plane(corner, bounds);
            min[0] = min[0].min(world.x);
            min[1] = min[1].min(world.y);
            max[0] = max[0].max(world.x);
            max[1] = max[1].max(world.y);
            any = true;
        }
        if !any {
            return Vec::new();
        }
        let visible = grip_points::visibility(&self.disabled_plugins);
        let candidates = self.tabs[i].plugin_grips.query_rect(min, max, &visible);
        let mut out = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if let Some(screen) = cam.project(candidate.point, bounds) {
                out.push((candidate, iced::Point::new(screen.x, screen.y)));
            }
        }
        out
    }

    /// Nearest visible plugin grip within the grip aperture of the
    /// tile-relative cursor — the custom-grip twin of `find_hit_grip`,
    /// feeding both hover and press. Tested before any wire pick, so a
    /// grip coincident with click-through (reference-only) geometry stays
    /// fully hittable and wins the hit.
    pub(in crate::app) fn hit_plugin_grip(&self, i: usize, cursor: Point) -> Option<GripCandidate> {
        let candidates = self.plugin_grips_in_view(i);
        let mut best_dist = crate::scene::pick::grip::GRIP_THRESHOLD_PX;
        let mut best = None;
        for (candidate, screen) in candidates {
            let dx = screen.x - cursor.x;
            let dy = screen.y - cursor.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d < best_dist {
                best_dist = d;
                best = Some(candidate);
            }
        }
        best
    }

    /// Press on a plugin-declared grip: engage the drag and deliver the begin
    /// event. The plugin's begin answer (if any) seeds the live preview; a
    /// missing owner (process died between render and press) is a no-op.
    pub(in crate::app) fn begin_plugin_grip_drag(
        &mut self,
        i: usize,
        candidate: &GripCandidate,
    ) -> Task<Message> {
        self.plugin_grip_moves = Default::default();
        let active = ActivePluginGrip {
            tab: i,
            plugin: candidate.plugin.clone(),
            grip_id: candidate.grip_id,
            anchor: candidate.anchor,
            origin_world: candidate.point,
            axis: candidate.axis,
            last_world: candidate.point,
            moved: false,
        };
        let event = active.event(
            ocs_plugin_api::host::CustomGripPhase::Begin,
            candidate.point,
        );
        let response = grip_points::dispatch_grip_event(self, i, &candidate.plugin, event);
        self.tabs[i].active_plugin_grip = Some(active);
        if let Some(response) = response {
            self.apply_plugin_grip_preview(i, Some(response));
        }
        // Keep pumping until the drag ends: answers to coalesced moves and
        // late previews must land even when the cursor rests.
        self.schedule_plugin_grip_pump()
    }

    /// Park the newest constrained cursor point for the active custom drag
    /// (the viewport move branch computes the point; the channel drops
    /// superseded ones) and drive delivery.
    pub(in crate::app) fn move_plugin_grip_drag(
        &mut self,
        i: usize,
        point: DVec3,
    ) -> Task<Message> {
        if let Some(active) = self.tabs[i].active_plugin_grip.as_mut() {
            active.last_world = point;
            active.moved = true;
        }
        self.plugin_grip_moves.park(point);
        self.pump_plugin_grips()
    }

    /// The pump: collect at most one outstanding move answer, send the newest
    /// parked point when the channel is free, and reschedule itself while the
    /// drag lives. Never blocks the app loop on plugin latency.
    pub(in crate::app) fn pump_plugin_grips(&mut self) -> Task<Message> {
        let Some(active) = self.active_plugin_grip() else {
            return Task::none();
        };
        let tab = active.tab;
        let plugin = active.plugin.clone();
        // In-process sinks deliver synchronously — coalescing holds trivially.
        if grip_points::sink_installed(&plugin) {
            while let Some(point) = self.plugin_grip_moves.take_sendable() {
                let event = self.tabs[tab]
                    .active_plugin_grip
                    .as_ref()
                    .map(|active| active.event(ocs_plugin_api::host::CustomGripPhase::Move, point));
                if let Some(event) = event {
                    let response = grip_points::dispatch_grip_event(self, tab, &plugin, event);
                    self.apply_plugin_grip_preview(tab, response);
                }
            }
            return self.schedule_plugin_grip_pump();
        }
        // Out-of-process: poll for the outstanding answer, then send the
        // newest parked point when the channel is free.
        if self.plugin_grip_moves.is_in_flight() {
            let response = grip_points::poll_grip_move(self, tab, &plugin, MOVE_POLL_SLICE);
            if let Some(response) = response {
                self.plugin_grip_moves.mark_resolved();
                self.apply_plugin_grip_preview(tab, Some(response));
            }
        }
        if let Some(point) = self.plugin_grip_moves.take_sendable() {
            let event = self.tabs[tab]
                .active_plugin_grip
                .as_ref()
                .map(|active| active.event(ocs_plugin_api::host::CustomGripPhase::Move, point));
            if let Some(event) = event {
                if grip_points::send_grip_move(self, tab, &plugin, event) {
                    self.plugin_grip_moves.mark_in_flight();
                }
            }
        }
        self.schedule_plugin_grip_pump()
    }

    /// Settle (mouse release, or Enter): drain the channel, deliver the final
    /// point, and let the plugin apply its edits inside its own undo scope.
    /// The settled drag totals exactly the plugin's one labeled undo step —
    /// the host records nothing of its own; the pending history the plugin
    /// opened is closed here (the command-boundary discipline), an empty one
    /// costs no step.
    pub(in crate::app) fn settle_plugin_grip_drag(&mut self) -> Task<Message> {
        let Some(active) = self.active_plugin_grip() else {
            return Task::none();
        };
        let tab = active.tab;
        let plugin = active.plugin.clone();
        let final_point = active.last_world;
        // Drain any outstanding move answer first — the settle is a real
        // (blocking, never-coalesced) call and must never meet a stale frame.
        self.drain_plugin_grip_moves(tab, &plugin, MOVE_DRAIN_BUDGET);
        let event = self.tabs[tab]
            .active_plugin_grip
            .as_ref()
            .map(|active| active.event(ocs_plugin_api::host::CustomGripPhase::Settle, final_point));
        if let Some(event) = event {
            let _ = grip_points::dispatch_grip_event(self, tab, &plugin, event);
        }
        // Close the plugin-opened pending history: one plugin-labeled step
        // when it wrote, zero when it did not.
        self.finish_pending_history(tab);
        self.end_plugin_grip_drag(tab);
        self.refresh_properties();
        Task::none()
    }

    /// Cancel (Esc, tab switch, another command starting, process death):
    /// deliver the cancel phase so the plugin drops preview state, and write
    /// nothing — zero undo steps, byte-identical drawing.
    pub(in crate::app) fn cancel_plugin_grip_drag(&mut self) -> Task<Message> {
        let Some(active) = self.active_plugin_grip() else {
            return Task::none();
        };
        let tab = active.tab;
        let plugin = active.plugin.clone();
        let last_point = active.last_world;
        self.drain_plugin_grip_moves(tab, &plugin, MOVE_DRAIN_BUDGET);
        let event = self.tabs[tab]
            .active_plugin_grip
            .as_ref()
            .map(|active| active.event(ocs_plugin_api::host::CustomGripPhase::Cancel, last_point));
        if let Some(event) = event {
            let _ = grip_points::dispatch_grip_event(self, tab, &plugin, event);
        }
        self.end_plugin_grip_drag(tab);
        Task::none()
    }

    /// The active custom drag (one at a time, app-wide).
    pub(in crate::app) fn active_plugin_grip(&self) -> Option<&ActivePluginGrip> {
        for tab in &self.tabs {
            if let Some(active) = &tab.active_plugin_grip {
                return Some(active);
            }
        }
        None
    }

    /// Shared end-of-drag cleanup: state off, preview off, tab marked dirty.
    fn end_plugin_grip_drag(&mut self, tab: usize) {
        self.tabs[tab].active_plugin_grip = None;
        self.plugin_grip_moves = Default::default();
        // The drag's live feedback ends with it: clear the plugin's preview
        // set whatever the last response was.
        self.tabs[tab].scene.set_preview_wires(Vec::new());
        self.tabs[tab].dirty = true;
    }

    /// Wait (bounded) for the outstanding coalesced move to be answered so
    /// settle/cancel never meet a stale response frame.
    fn drain_plugin_grip_moves(&mut self, tab: usize, plugin: &str, budget: Duration) {
        if !self.plugin_grip_moves.is_in_flight() {
            return;
        }
        if grip_points::sink_installed(plugin) {
            self.plugin_grip_moves.mark_resolved();
            return;
        }
        let deadline = std::time::Instant::now() + budget;
        while self.plugin_grip_moves.is_in_flight() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match grip_points::poll_grip_move(self, tab, plugin, remaining.min(MOVE_POLL_SLICE)) {
                Some(response) => {
                    self.plugin_grip_moves.mark_resolved();
                    self.apply_plugin_grip_preview(tab, Some(response));
                }
                None => {
                    if !grip_points::grip_move_in_flight(plugin) {
                        // The connection died or the move was already answered
                        // elsewhere: stop waiting, never spin.
                        self.plugin_grip_moves.mark_resolved();
                        break;
                    }
                }
            }
        }
    }

    /// Apply the plugin's latest preview set (REPLACE semantics — an absent
    /// response clears it).
    pub(in crate::app) fn apply_plugin_grip_preview(
        &mut self,
        tab: usize,
        response: Option<ocs_plugin_api::host::CustomGripResponse>,
    ) {
        let wires = response.map(|r| crate::app::plugin_host::convert_preview_wires(r.preview));
        if let Some(wires) = wires {
            self.tabs[tab].scene.set_preview_wires(wires);
            self.tabs[tab].dirty = true;
        }
    }

    fn schedule_plugin_grip_pump(&mut self) -> Task<Message> {
        if self.active_plugin_grip().is_some() {
            Task::perform(grip_pump_delay(), |()| Message::PluginGripPump)
        } else {
            Task::none()
        }
    }

    /// Reap the grip sets of plugins that died or were unloaded, and cancel
    /// the drag/hover if the dying plugin owned it. Called from the periodic
    /// plugin drain tick; nothing else changes ("removes the grip and
    /// nothing else"). Liveness is the same visibility the render/hit paths
    /// apply (loaded, alive, not disabled).
    pub(in crate::app) fn reap_plugin_grips(&mut self) {
        let (dying_drag, dead_hover) = {
            let visible = grip_points::visibility(&self.disabled_plugins);
            for i in 0..self.tabs.len() {
                self.tabs[i]
                    .plugin_grips
                    .retain_alive(|plugin| visible(plugin));
            }
            let dying_drag = self
                .active_plugin_grip()
                .map(|active| !visible(&active.plugin))
                .unwrap_or(false);
            let dead_hover = self
                .plugin_grip_hover
                .as_ref()
                .is_some_and(|hover| !visible(&hover.plugin));
            (dying_drag, dead_hover)
        };
        if dying_drag {
            let _ = self.cancel_plugin_grip_drag();
        }
        if dead_hover {
            self.plugin_grip_hover = None;
        }
    }
}
