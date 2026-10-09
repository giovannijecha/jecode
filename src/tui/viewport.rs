use super::line::{Line, Origin};

#[derive(Clone, Copy)]
pub(super) enum Scroll {
    Rows(isize),
    Page(bool),
    Start,
    End,
    Reveal(usize),
}

#[derive(Clone, Copy)]
struct Anchor {
    block: usize,
    row: usize,
    origin: Option<Origin>,
}

#[derive(Default)]
pub(super) struct Viewport {
    anchor: Option<Anchor>,
    requests: Vec<Scroll>,
}

pub(super) struct Slice {
    pub lines: Vec<Line>,
    pub reading: bool,
}

impl Viewport {
    pub fn following(&self) -> bool {
        self.anchor.is_none()
    }
    pub fn scroll(&mut self, request: Scroll) {
        if matches!(request, Scroll::Rows(0)) {
            return;
        }
        if let Scroll::Rows(amount) = request
            && let Some(Scroll::Rows(previous)) = self.requests.last_mut()
        {
            *previous = previous.saturating_add(amount);
        } else {
            self.requests.push(request);
        }
    }

    pub fn view(&mut self, blocks: &[Vec<Line>], pending: &[Vec<Line>], height: usize) -> Slice {
        if height == 0 {
            return Slice {
                lines: vec![],
                reading: false,
            };
        }
        let blocks: Vec<_> = blocks.iter().chain(pending).collect();
        let mut starts = Vec::with_capacity(blocks.len() + 1);
        starts.push(0usize);
        for block in &blocks {
            starts.push(starts.last().unwrap().saturating_add(block.len()));
        }
        let total = *starts.last().unwrap();
        let end = total.saturating_sub(height);
        let mut top = self
            .anchor
            .map_or(end, |anchor| {
                let Some(block) = blocks.get(anchor.block) else {
                    return end;
                };
                let row = anchor.origin.map_or(anchor.row, |origin| {
                    if block.get(anchor.row).and_then(|line| line.origin) == Some(origin) {
                        return anchor.row;
                    }
                    block
                        .iter()
                        .enumerate()
                        .filter_map(|(row, line)| {
                            line.origin
                                .filter(|candidate| *candidate <= origin)
                                .map(|_| row)
                        })
                        .next_back()
                        .unwrap_or(0)
                });
                starts[anchor.block] + row.min(block.len().saturating_sub(1))
            })
            .min(total.saturating_sub(1));
        let mut following = self.anchor.is_none();
        for request in self.requests.drain(..) {
            match request {
                Scroll::Rows(amount) => {
                    top = top.saturating_add_signed(amount).min(end);
                    following = amount > 0 && top == end;
                }
                Scroll::Page(up) => {
                    let page = height.saturating_sub(1).max(1);
                    top = if up {
                        top.saturating_sub(page)
                    } else {
                        top.saturating_add(page).min(end)
                    };
                    following = !up && top == end;
                }
                Scroll::Start => {
                    top = 0;
                    following = false;
                }
                Scroll::End => {
                    top = end;
                    following = true;
                }
                Scroll::Reveal(block) => {
                    if let Some(start) = starts.get(block).copied() {
                        if start < top || start >= top + height {
                            top = start.min(end);
                        }
                        following = false;
                    }
                }
            }
        }
        // A short conversation, a resize, or the last downward step can put
        // the anchored page at the actual bottom. Resume following there.
        if top == end {
            following = true;
        }
        let first = starts
            .partition_point(|start| *start <= top)
            .saturating_sub(1);
        let mut lines = Vec::with_capacity(height);
        for (index, block) in blocks.iter().enumerate().skip(first) {
            let offset = top.saturating_sub(starts[index]);
            let left = height.saturating_sub(lines.len());
            lines.extend(block.iter().skip(offset).take(left).cloned());
            if lines.len() == height {
                break;
            }
        }
        if height > 0 && !following && first < blocks.len() {
            let row = top - starts[first];
            self.anchor = Some(Anchor {
                block: first,
                row,
                origin: blocks[first].get(row).and_then(|line| line.origin),
            });
        } else if following {
            self.anchor = None;
        }
        Slice {
            lines,
            reading: self.anchor.is_some(),
        }
    }
}

#[cfg(test)]
mod tests;
