//! The scopes that presence walks, and what each one binds. A plain assignment that starts a name is a candidate for
//! exemption, and it is settled when its scope closes, because only then is it known whether the name is bound again.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ruff_text_size::TextSize;

#[derive(Debug, Clone)]
pub enum ScopeKind {
    Module,
    Class(ClassScope),
    /// A function or lambda body. `receiver` is set for a method body, where attributes are judged.
    Function {
        receiver: Option<Receiver>,
    },
}

#[derive(Debug, Clone)]
pub struct ClassScope {
    pub name: String,
    /// Where the class statement starts, which identifies the class in the class index.
    pub start: TextSize,
    /// Whether the class is an `Enum` class, whose plain assignments are members, not attributes.
    pub is_enum: bool,
    /// The attributes that the class declares, or `None` when an ancestor is external.
    pub declared: Option<Rc<HashSet<String>>>,
}

#[derive(Debug, Clone)]
pub struct Receiver {
    pub name: String,
    /// The class that the method belongs to, for messages.
    pub class: String,
    pub start: TextSize,
    /// The attributes that the class declares, or `None` when an ancestor is external. Then no attribute is
    /// judged, but the assignments are still recorded for the class body.
    pub declared: Option<Rc<HashSet<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    Global,
    Nonlocal,
}

/// A plain assignment that starts its name in its scope, and may be exempt from an annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub offset: TextSize,
}

/// A candidate after its scope closed. `rebound` is the earliest later binding of the name, if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub candidate: Candidate,
    pub rebound: Option<TextSize>,
}

pub struct Closed {
    pub kind: ScopeKind,
    pub settled: Vec<Settled>,
}

/// What a scope must take in from a scope inside it: a name that a nested function binds through `global` or
/// `nonlocal`, or an attribute that a method assigns through its receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    Escaped(Escape),
    Attribute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    name: String,
    offset: TextSize,
    notice: Notice,
}

#[derive(Debug, Clone, Copy)]
struct Bound {
    first: TextSize,
    rebound: Option<TextSize>,
}

impl Bound {
    /// Records a binding at `offset`. Only a binding other than the first one can be a rebinding.
    fn note(&mut self, offset: TextSize) {
        if offset != self.first {
            self.rebound = Some(self.rebound.map_or(offset, |earliest| earliest.min(offset)));
        }
    }
}

#[derive(Debug)]
struct Scope {
    kind: ScopeKind,
    bound: HashMap<String, Bound>,
    declared: HashMap<String, Escape>,
    candidates: Vec<Candidate>,
    notices: Vec<Event>,
}

impl Scope {
    fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            bound: HashMap::new(),
            declared: HashMap::new(),
            candidates: Vec::new(),
            notices: Vec::new(),
        }
    }

    fn note(&mut self, name: &str, offset: TextSize) {
        match self.bound.get_mut(name) {
            Some(bound) => bound.note(offset),
            None => {
                self.bound.insert(
                    name.to_owned(),
                    Bound {
                        first: offset,
                        rebound: None,
                    },
                );
            }
        }
    }

    fn is_bound(&self, name: &str) -> bool {
        self.bound.contains_key(name) || self.declared.contains_key(name)
    }

    /// Takes in a notice that this scope binds. A notice this scope does not take is returned, so that the
    /// scope around it can take it. An attribute notice never travels further.
    fn take(&mut self, event: Event) -> Option<Event> {
        if self.accepts(event.notice)
            && let Some(bound) = self.bound.get_mut(&event.name)
        {
            bound.note(event.offset);
            return None;
        }
        match event.notice {
            Notice::Escaped(_) => Some(event),
            Notice::Attribute => None,
        }
    }

    /// A class body is skipped by a `nonlocal`, and a `global` binds only the module.
    fn accepts(&self, notice: Notice) -> bool {
        matches!(
            (notice, &self.kind),
            (Notice::Attribute, ScopeKind::Class(_))
                | (Notice::Escaped(Escape::Global), ScopeKind::Module)
                | (
                    Notice::Escaped(Escape::Nonlocal),
                    ScopeKind::Module | ScopeKind::Function { .. }
                )
        )
    }

    /// Takes in the notices that reached this scope, passes the rest to `outer`, and returns the candidates
    /// with their rebinding. Nothing can reach the scope after this.
    fn settle(&mut self, outer: Option<&mut Scope>) -> Vec<Settled> {
        let notices = std::mem::take(&mut self.notices);
        let passed: Vec<Event> = notices
            .into_iter()
            .filter_map(|event| self.take(event))
            .collect();
        if let Some(outer) = outer {
            outer.notices.extend(passed);
        }
        self.candidates
            .drain(..)
            .map(|candidate| {
                let rebound = self
                    .bound
                    .get(&candidate.name)
                    .and_then(|bound| bound.rebound);
                Settled { candidate, rebound }
            })
            .collect()
    }
}

/// The innermost scope is kept apart from the enclosing ones, so the stack always has a current scope.
#[derive(Debug)]
pub struct ScopeStack {
    current: Scope,
    enclosing: Vec<Scope>,
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self {
            current: Scope::new(ScopeKind::Module),
            enclosing: Vec::new(),
        }
    }
}

impl ScopeStack {
    pub fn push(&mut self, kind: ScopeKind) {
        let outer = std::mem::replace(&mut self.current, Scope::new(kind));
        self.enclosing.push(outer);
    }

    /// The module scope is closed last, and then nothing is passed on.
    pub fn close(&mut self) -> Closed {
        match self.enclosing.pop() {
            Some(outer) => {
                let mut inner = std::mem::replace(&mut self.current, outer);
                let settled = inner.settle(Some(&mut self.current));
                Closed {
                    kind: inner.kind,
                    settled,
                }
            }
            None => {
                let mut module =
                    std::mem::replace(&mut self.current, Scope::new(ScopeKind::Module));
                let settled = module.settle(None);
                Closed {
                    kind: module.kind,
                    settled,
                }
            }
        }
    }

    /// Records that `name` is bound at `offset` in the current scope. A name that the scope declares `global`
    /// or `nonlocal` is bound in the scope it refers to, so the binding is passed on.
    pub fn bind(&mut self, name: &str, offset: TextSize) {
        match self.current.declared.get(name).copied() {
            Some(escape) => self.notify_outer(name, offset, Notice::Escaped(escape)),
            None => self.current.note(name, offset),
        }
    }

    pub fn declare(&mut self, name: &str, escape: Escape) {
        self.current.declared.insert(name.to_owned(), escape);
    }

    pub fn note_receiver_assignment(&mut self, attribute: &str, offset: TextSize) {
        self.notify_outer(attribute, offset, Notice::Attribute);
    }

    pub fn add_candidate(&mut self, name: &str, offset: TextSize) {
        self.current.candidates.push(Candidate {
            name: name.to_owned(),
            offset,
        });
    }

    /// Whether `name` is already bound in the current scope, including by a `global` or `nonlocal` declaration.
    pub fn is_bound(&self, name: &str) -> bool {
        self.current.is_bound(name)
    }

    pub fn current_kind(&self) -> &ScopeKind {
        &self.current.kind
    }

    fn notify_outer(&mut self, name: &str, offset: TextSize, notice: Notice) {
        if let Some(outer) = self.enclosing.last_mut() {
            outer.notices.push(Event {
                name: name.to_owned(),
                offset,
                notice,
            });
        }
    }
}
