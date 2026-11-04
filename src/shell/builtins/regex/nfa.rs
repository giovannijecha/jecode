//! Thompson-style states. One state keeps the earliest start that reached it;
//! an earlier start always dominates a later one with the same future path.

use super::{Boundary, Class, Item, Node, decode, previous, word};
use std::collections::VecDeque;

const MAX_STATES: usize = 100_000;
const NONE: usize = usize::MAX;

pub(super) struct Machine {
    states: Vec<State>,
    start: usize,
    accept: usize,
}

#[derive(Default)]
struct State {
    edges: Vec<Edge>,
}

struct Edge {
    target: usize,
    kind: Kind,
}

enum Kind {
    Empty,
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    Boundary(Boundary),
}

/// What zero-width assertions can see at one place in the text.
struct Position {
    first: bool,
    before: Option<char>,
    after: Option<char>,
}

struct Fragment {
    start: usize,
    end: usize,
}

impl Machine {
    pub(super) fn compile(root: &Node) -> Result<Self, String> {
        let mut machine = Self {
            states: Vec::new(),
            start: 0,
            accept: 0,
        };
        let fragment = machine.node(root)?;
        machine.start = fragment.start;
        machine.accept = fragment.end;
        Ok(machine)
    }

    pub(super) fn find(
        &self,
        text: &[u8],
        from: usize,
        insensitive: bool,
    ) -> Option<(usize, usize)> {
        if from > text.len() {
            return None;
        }
        let mut active = vec![NONE; self.states.len()];
        let mut queue = VecDeque::new();
        let mut best: Option<(usize, usize)> = None;
        let mut before = previous(text, from);
        let mut at = from;
        loop {
            let current = decode(text, at);
            Self::add(&mut active, &mut queue, self.start, at);
            let position = Position {
                first: at == 0,
                before,
                after: current.map(|(ch, _)| ch),
            };
            self.closure(&mut active, &mut queue, &position);
            let start = active[self.accept];
            if start != NONE && best.is_none_or(|(old, _)| start <= old) {
                best = Some((start, at));
            }
            if let Some((start, _)) = best
                && active.iter().all(|active_start| *active_start > start)
            {
                return best;
            }
            let Some((ch, length)) = current else { break };
            let mut next = vec![NONE; self.states.len()];
            for (state, start) in active.iter().copied().enumerate() {
                if start == NONE {
                    continue;
                }
                for edge in &self.states[state].edges {
                    if edge.kind.consumes(ch, insensitive) {
                        Self::add(&mut next, &mut queue, edge.target, start);
                    }
                }
            }
            active = next;
            before = Some(ch);
            at += length;
        }
        best
    }

    fn closure(&self, active: &mut [usize], queue: &mut VecDeque<usize>, position: &Position) {
        while let Some(state) = queue.pop_front() {
            let start = active[state];
            for edge in &self.states[state].edges {
                let passes = match edge.kind {
                    Kind::Empty => true,
                    Kind::Start => position.first,
                    Kind::End => position.after.is_none(),
                    Kind::Boundary(boundary) => {
                        let before = position.before.is_some_and(word);
                        let after = position.after.is_some_and(word);
                        match boundary {
                            Boundary::Word(wanted) => (before != after) == wanted,
                            Boundary::Start => !before && after,
                            Boundary::End => before && !after,
                            Boundary::NoWordBefore => !before,
                            Boundary::NoWordAfter => !after,
                        }
                    }
                    _ => false,
                };
                if passes {
                    Self::add(active, queue, edge.target, start);
                }
            }
        }
    }

    fn add(active: &mut [usize], queue: &mut VecDeque<usize>, state: usize, start: usize) {
        if start < active[state] {
            active[state] = start;
            queue.push_back(state);
        }
    }

    fn state(&mut self) -> Result<usize, String> {
        if self.states.len() == MAX_STATES {
            return Err("regular expression has too many states".into());
        }
        let index = self.states.len();
        self.states.push(State::default());
        Ok(index)
    }

    fn edge(&mut self, from: usize, target: usize, kind: Kind) {
        self.states[from].edges.push(Edge { target, kind });
    }

    fn atom(&mut self, kind: Kind) -> Result<Fragment, String> {
        let start = self.state()?;
        let end = self.state()?;
        self.edge(start, end, kind);
        Ok(Fragment { start, end })
    }

    fn node(&mut self, node: &Node) -> Result<Fragment, String> {
        match node {
            Node::Char(ch) => self.atom(Kind::Char(*ch)),
            Node::Any => self.atom(Kind::Any),
            Node::Class(class) => self.atom(Kind::Class(class.clone())),
            Node::Start => self.atom(Kind::Start),
            Node::End => self.atom(Kind::End),
            Node::Boundary(wanted) => self.atom(Kind::Boundary(*wanted)),
            Node::Sequence(nodes) => {
                let mut result = self.atom(Kind::Empty)?;
                for node in nodes {
                    let next = self.node(node)?;
                    self.edge(result.end, next.start, Kind::Empty);
                    result.end = next.end;
                }
                Ok(result)
            }
            Node::Alternatives(nodes) => {
                let start = self.state()?;
                let end = self.state()?;
                for node in nodes {
                    let option = self.node(node)?;
                    self.edge(start, option.start, Kind::Empty);
                    self.edge(option.end, end, Kind::Empty);
                }
                Ok(Fragment { start, end })
            }
            Node::Repeat(inner, min, max) => self.repeat(inner, *min, *max),
        }
    }

    fn repeat(&mut self, inner: &Node, min: usize, max: Option<usize>) -> Result<Fragment, String> {
        if max.is_some_and(|max| max < min) {
            return Err("invalid regular expression repetition bounds".into());
        }
        let start = self.state()?;
        let mut end = start;
        for _ in 0..min {
            let copy = self.node(inner)?;
            self.edge(end, copy.start, Kind::Empty);
            end = copy.end;
        }
        if let Some(max) = max {
            for _ in min..max {
                let after = self.state()?;
                let copy = self.node(inner)?;
                self.edge(end, after, Kind::Empty);
                self.edge(end, copy.start, Kind::Empty);
                self.edge(copy.end, after, Kind::Empty);
                end = after;
            }
        } else {
            let after = self.state()?;
            let copy = self.node(inner)?;
            self.edge(end, after, Kind::Empty);
            self.edge(end, copy.start, Kind::Empty);
            self.edge(copy.end, end, Kind::Empty);
            end = after;
        }
        Ok(Fragment { start, end })
    }
}

impl Kind {
    fn consumes(&self, ch: char, insensitive: bool) -> bool {
        match self {
            Self::Char(expected) => {
                ch == *expected || insensitive && ch.to_lowercase().eq(expected.to_lowercase())
            }
            Self::Any => true,
            Self::Class(class) => {
                let test = |ch: char| {
                    class.items.iter().any(|item| match item {
                        Item::Range(low, high) => (*low..=*high).contains(&ch),
                        Item::Named(test, negated) => test(ch) != *negated,
                    })
                };
                let found = test(ch)
                    || insensitive && (ch.to_lowercase().any(test) || ch.to_uppercase().any(test));
                found != class.negated
            }
            _ => false,
        }
    }
}
