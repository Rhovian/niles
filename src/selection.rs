use anyhow::{Result, ensure};
use std::{collections::VecDeque, iter, mem};

/// A nonempty selection. Moving the focus transfers values, so no index can outlive its list.
/// VecDeque supplies ends, not selection: https://doc.rust-lang.org/1.89.0/std/collections/struct.VecDeque.html
pub(crate) struct Selection<T> {
    before: VecDeque<T>,
    current: T,
    after: VecDeque<T>,
}

impl<T> Selection<T> {
    pub(crate) fn new(values: impl IntoIterator<Item = T>) -> Option<Self> {
        let mut values = values.into_iter();
        Some(Self {
            current: values.next()?,
            before: VecDeque::new(),
            after: values.collect(),
        })
    }

    pub(crate) fn current(&self) -> &T {
        &self.current
    }

    pub(crate) fn current_mut(&mut self) -> &mut T {
        &mut self.current
    }

    pub(crate) fn position(&self) -> usize {
        self.before.len()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.before
            .iter()
            .rev()
            .chain(iter::once(&self.current))
            .chain(&self.after)
    }

    pub(crate) fn focus(&mut self, position: usize) -> Result<()> {
        ensure!(
            self.iter().nth(position).is_some(),
            "selection position {position} is missing"
        );
        while self.position() != position {
            self.move_focus(self.position() < position);
        }
        Ok(())
    }

    #[expect(clippy::disallowed_methods, reason = "menu boundary")]
    #[expect(clippy::expect_used, reason = "index derived from owned menu")]
    pub(crate) fn prefer(&mut self, predicate: impl Fn(&T) -> bool) {
        let position = self.iter().position(predicate).unwrap_or(0);
        self.focus(position).expect("index derived from owned menu");
    }

    pub(crate) fn move_focus(&mut self, forward: bool) {
        let (from, to) = if forward {
            (&mut self.after, &mut self.before)
        } else {
            (&mut self.before, &mut self.after)
        };
        if let Some(value) = from.pop_front() {
            to.push_front(mem::replace(&mut self.current, value));
        }
    }

    pub(crate) fn move_matching(&mut self, forward: bool, eligible: impl Fn(&T) -> bool) {
        let side = if forward { &self.after } else { &self.before };
        let steps = side.iter().position(eligible);
        if let Some(steps) = steps {
            for _ in 0..=steps {
                self.move_focus(forward);
            }
        }
    }

    pub(crate) fn insert_after(&mut self, value: T) {
        self.after.push_front(value);
    }

    pub(crate) fn remove(&mut self) -> Result<()> {
        let next = self.before.pop_front().or_else(|| self.after.pop_front());
        self.current = next.ok_or_else(|| anyhow::anyhow!("cannot remove the last selection"))?;
        Ok(())
    }

    pub(crate) fn reorder(&mut self, forward: bool, eligible: impl FnOnce(&T) -> bool) {
        let (from, to) = if forward {
            (&mut self.after, &mut self.before)
        } else {
            (&mut self.before, &mut self.after)
        };
        if from.front().is_some_and(eligible)
            && let Some(value) = from.pop_front()
        {
            to.push_front(value);
        }
    }
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::cast_sign_loss,
    reason = "pointer-sized operands widen to i128; remainder is nonnegative"
)]
pub(crate) fn cycle<T>(items: &[T], index: Option<usize>, steps: isize) -> Option<&T> {
    let Some(index) = index else {
        return items.first();
    };
    items.get(index)?;
    let next = (index as i128 + steps as i128).checked_rem_euclid(items.len() as i128)?;
    items.get(next as usize)
}
