//! Main thread event emitter (port of Node's EventEmitter usage).
//! Listeners receive `&mut App` so they can reach any service, exactly like the
//! original closures captured the service host.

use std::rc::Rc;

use crate::app::App;

pub type ListenerId = u64;
pub type Callback<T> = Rc<dyn Fn(&mut App, &T)>;

pub struct Emitter<T: ?Sized> {
    listeners: Vec<(ListenerId, bool, Callback<T>)>,
    next_id: ListenerId,
}

impl<T: ?Sized> Default for Emitter<T> {
    fn default() -> Self {
        Self { listeners: Vec::new(), next_id: 1 }
    }
}

impl<T: ?Sized + 'static> Emitter<T> {
    pub fn on(&mut self, f: impl Fn(&mut App, &T) + 'static) -> ListenerId {
        self.add(Rc::new(f), false)
    }

    pub fn once(&mut self, f: impl Fn(&mut App, &T) + 'static) -> ListenerId {
        self.add(Rc::new(f), true)
    }

    fn add(&mut self, cb: Callback<T>, once: bool) -> ListenerId {
        let id = self.next_id;
        self.next_id += 1;
        self.listeners.push((id, once, cb));
        id
    }

    pub fn off(&mut self, id: ListenerId) {
        self.listeners.retain(|(i, _, _)| *i != id);
    }

    pub fn clear(&mut self) {
        self.listeners.clear();
    }

    pub fn len(&self) -> usize {
        self.listeners.len()
    }

    /// Snapshot of listeners for an emit; once listeners are removed.
    pub fn take_for_emit(&mut self) -> Vec<Callback<T>> {
        let cbs = self.listeners.iter().map(|(_, _, cb)| cb.clone()).collect();
        self.listeners.retain(|(_, once, _)| !once);
        cbs
    }
}

pub fn run_callbacks<T: ?Sized>(app: &mut App, callbacks: Vec<Callback<T>>, value: &T) {
    for cb in callbacks {
        cb(app, value);
    }
}

/// `emit!(app, app.some.emitter, &value)`; `value` must not borrow `app`.
#[macro_export]
macro_rules! emit {
    ($app:expr, $emitter:expr, $value:expr) => {{
        let __callbacks = $emitter.take_for_emit();
        $crate::events::run_callbacks($app, __callbacks, $value);
    }};
}
