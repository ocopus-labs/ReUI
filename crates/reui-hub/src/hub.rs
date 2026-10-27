//! The hub's shared state: every module's live activities, and which one is in front.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use reui_protocol::{Activity, ActivityId, CompactView, Element, ExpandedView, ImageId, Priority, Rgb, Snapshot};
use tokio::sync::{mpsc, oneshot, watch};

/// How an activity presents itself; see the protocol for each presentation.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub accent: Option<Rgb>,
    pub compact: CompactView,
    pub minimal: Element,
    pub expanded: Option<ExpandedView>,
}

/// An action a surface sent back for one of a module's activities.
#[derive(Debug)]
pub struct ModuleAction {
    pub key: String,
    pub action: String,
}

/// A coding agent's hook event, delivered by `reui-bridge`.
#[derive(Debug)]
pub struct HookCall {
    /// Unique per call; [`ModuleInput::HookGone`] refers to it.
    pub id: u64,
    /// The hook's stdin JSON, verbatim.
    pub payload: String,
    /// Present when the agent is waiting for an answer (permission requests). Send `None`
    /// for "no decision"; dropping it has the same effect.
    pub reply: Option<oneshot::Sender<Option<String>>>,
}

/// Everything a module can receive.
#[derive(Debug)]
pub enum ModuleInput {
    Action(ModuleAction),
    Hook(HookCall),
    /// The bridge waiting on this hook call went away (the agent was answered in its own
    /// UI, timed out, or was closed); drop whatever was shown for it.
    HookGone(u64),
}

struct Entry {
    activity: Activity,
    /// Order of first publish; newer wins among equal priorities.
    seq: u64,
}

#[derive(Default)]
struct State {
    entries: HashMap<ActivityId, Entry>,
    routes: HashMap<String, mpsc::UnboundedSender<ModuleInput>>,
    /// Encoded images by content hash; surfaces fetch them once, before first use.
    images: HashMap<ImageId, Arc<Vec<u8>>>,
    next_seq: u64,
    next_hook: u64,
}

#[derive(Clone)]
pub struct Hub {
    state: Arc<Mutex<State>>,
    snapshots: Arc<watch::Sender<Arc<Snapshot>>>,
}

impl Hub {
    pub fn new() -> Self {
        let (snapshots, _) = watch::channel(Arc::new(Snapshot::default()));
        Self { state: Arc::default(), snapshots: Arc::new(snapshots) }
    }

    /// Latest snapshot plus change notifications. `watch` keeps only the newest value,
    /// so a slow surface skips intermediate states instead of queueing them.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshots.subscribe()
    }

    pub fn context(&self, module: &'static str) -> ModuleContext {
        let (tx, inputs) = mpsc::unbounded_channel();
        self.state.lock().unwrap().routes.insert(module.to_string(), tx);
        ModuleContext { module, hub: self.clone(), inputs }
    }

    pub fn image(&self, id: ImageId) -> Option<Arc<Vec<u8>>> {
        self.state.lock().unwrap().images.get(&id).cloned()
    }

    fn add_image(&self, bytes: Vec<u8>) -> ImageId {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let id = ImageId(hasher.finish());
        self.state.lock().unwrap().images.entry(id).or_insert_with(|| Arc::new(bytes));
        id
    }

    pub fn route_action(&self, id: &ActivityId, action: String) {
        let state = self.state.lock().unwrap();
        if !state.entries.contains_key(id) {
            return; // stale: the activity was retracted while the user was clicking
        }
        if let Some(route) = state.routes.get(&id.module) {
            _ = route.send(ModuleInput::Action(ModuleAction { key: id.key.clone(), action }));
        }
    }

    /// Hands a hook event to the module named `agent`. Returns the call id, or `None` if no
    /// such module is running (the bridge then answers "no decision").
    pub fn route_hook(&self, agent: &str, payload: String, reply: Option<oneshot::Sender<Option<String>>>) -> Option<u64> {
        let mut state = self.state.lock().unwrap();
        state.next_hook += 1;
        let id = state.next_hook;
        let route = state.routes.get(agent)?;
        route.send(ModuleInput::Hook(HookCall { id, payload, reply })).ok()?;
        Some(id)
    }

    pub fn hook_gone(&self, agent: &str, id: u64) {
        if let Some(route) = self.state.lock().unwrap().routes.get(agent) {
            _ = route.send(ModuleInput::HookGone(id));
        }
    }

    fn publish(&self, activity: Activity) {
        let mut state = self.state.lock().unwrap();
        let seq = match state.entries.get(&activity.id) {
            Some(existing) => existing.seq,
            None => {
                state.next_seq += 1;
                state.next_seq
            }
        };
        state.entries.insert(activity.id.clone(), Entry { activity, seq });
        self.rebuild(&state);
    }

    fn retract(&self, id: &ActivityId) {
        let mut state = self.state.lock().unwrap();
        if state.entries.remove(id).is_some() {
            self.rebuild(&state);
        }
    }

    fn remove_module(&self, module: &str) {
        let mut state = self.state.lock().unwrap();
        state.routes.remove(module);
        let before = state.entries.len();
        state.entries.retain(|id, _| id.module != module);
        if state.entries.len() != before {
            self.rebuild(&state);
        }
    }

    /// Re-sort and publish, but only wake surfaces if what they'd see actually changed.
    fn rebuild(&self, state: &State) {
        let mut entries: Vec<&Entry> = state.entries.values().collect();
        entries.sort_by(|a, b| b.activity.priority.cmp(&a.activity.priority).then(b.seq.cmp(&a.seq)));
        let snapshot = Snapshot { activities: entries.into_iter().map(|e| e.activity.clone()).collect() };
        self.snapshots.send_if_modified(|current| {
            if **current == snapshot {
                return false;
            }
            *current = Arc::new(snapshot);
            true
        });
    }
}

/// A module's handle to the hub. Dropping it retracts all of the module's activities.
pub struct ModuleContext {
    module: &'static str,
    hub: Hub,
    inputs: mpsc::UnboundedReceiver<ModuleInput>,
}

impl ModuleContext {
    pub fn publish(&self, key: &str, priority: Priority, view: View) {
        let id = ActivityId { module: self.module.to_string(), key: key.to_string() };
        let View { accent, compact, minimal, expanded } = view;
        self.hub.publish(Activity { id, priority, accent, compact, minimal, expanded });
    }

    /// Registers encoded image bytes (PNG, JPEG, ...) for use as `Art::Image`.
    pub fn image(&self, bytes: Vec<u8>) -> ImageId {
        self.hub.add_image(bytes)
    }

    pub fn retract(&self, key: &str) {
        self.hub.retract(&ActivityId { module: self.module.to_string(), key: key.to_string() });
    }

    /// Cancel-safe; `None` once the hub is shutting down.
    pub async fn next(&mut self) -> Option<ModuleInput> {
        self.inputs.recv().await
    }
}

impl Drop for ModuleContext {
    fn drop(&mut self) {
        self.hub.remove_module(self.module);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compact(text: &str) -> View {
        let trailing = Element::Text { text: text.into(), tone: Default::default() };
        View { compact: CompactView { leading: Element::Dot, trailing }, ..Default::default() }
    }

    fn front(hub: &Hub) -> Vec<String> {
        hub.subscribe().borrow().activities.iter().map(|a| a.id.key.clone()).collect()
    }

    #[test]
    fn decisions_jump_ahead_and_newest_wins_ties() {
        let hub = Hub::new();
        let ctx = hub.context("m");
        ctx.publish("music", Priority::Ongoing, compact("a"));
        ctx.publish("timer", Priority::Ongoing, compact("b"));
        ctx.publish("ask", Priority::Decision, compact("c"));
        assert_eq!(front(&hub), ["ask", "timer", "music"]);

        // Updating an existing activity keeps its place.
        ctx.publish("music", Priority::Ongoing, compact("a2"));
        assert_eq!(front(&hub), ["ask", "timer", "music"]);

        ctx.retract("ask");
        assert_eq!(front(&hub), ["timer", "music"]);
    }

    #[test]
    fn identical_publish_does_not_wake_surfaces() {
        let hub = Hub::new();
        let ctx = hub.context("m");
        ctx.publish("music", Priority::Ongoing, compact("a"));
        let mut rx = hub.subscribe();
        rx.mark_unchanged();
        ctx.publish("music", Priority::Ongoing, compact("a"));
        assert!(!rx.has_changed().unwrap());
        ctx.publish("music", Priority::Ongoing, compact("b"));
        assert!(rx.has_changed().unwrap());
    }

    #[test]
    fn dropping_a_module_retracts_its_activities_and_actions_route() {
        let hub = Hub::new();
        let mut ctx = hub.context("m");
        let other = hub.context("o");
        ctx.publish("x", Priority::Ongoing, compact("a"));
        other.publish("y", Priority::Ongoing, compact("b"));

        hub.route_action(&ActivityId { module: "m".into(), key: "x".into() }, "go".into());
        let Ok(ModuleInput::Action(action)) = ctx.inputs.try_recv() else { panic!("expected an action") };
        assert_eq!((action.key.as_str(), action.action.as_str()), ("x", "go"));

        drop(ctx);
        assert_eq!(front(&hub), ["y"]);
    }
}
