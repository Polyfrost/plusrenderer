use std::cmp::Reverse;

use crate::signals::{is_bool_signal, SIGNALS};

use super::sim::SignalSample;

pub const OFF_SAMPLE: usize = usize::MAX;

const BOOL_SAMPLES: [f32; 2] = [0.0, 1.0];
const TIME_SAMPLES: [f32; 6] = [0.0, 0.25, 0.5, 1.0, 2.0, 5.0];

fn value_samples(id: usize) -> &'static [f32] {
    match SIGNALS.get(id).map(|(name, _)| *name) {
        Some("ground_speed") => &[0.0, 1.3, 4.3, 5.6, 10.0, 30.0],
        Some("vertical_speed") => &[0.0, -1.0, 1.0, 8.4, -20.0, -78.4],
        Some("time_of_day") => &[0.25, 0.0, 0.5, 0.75, 0.99],
        Some("biome_temperature") => &[0.8, -0.7, 0.0, 0.15, 0.5, 1.0, 2.0],
        Some("dimension") => &[0.0, 1.0, 2.0, -1.0],
        _ if is_bool_signal(id) => &BOOL_SAMPLES,
        _ => &[0.0, 0.5, 1.0, 10.0],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lane {
    pub id: usize,
    pub time: bool,
    pub samples: &'static [f32],
}

impl Lane {
    pub fn value(id: usize) -> Lane {
        Lane {
            id,
            time: false,
            samples: value_samples(id),
        }
    }

    pub fn time(id: usize) -> Lane {
        Lane {
            id,
            time: true,
            samples: &TIME_SAMPLES,
        }
    }

    pub fn index_of(&self, value: f32) -> usize {
        self.samples
            .iter()
            .position(|s| *s == value)
            .unwrap_or(OFF_SAMPLE)
    }
}

pub fn lanes(values: &[usize], times: &[usize]) -> Vec<Lane> {
    values
        .iter()
        .map(|id| Lane::value(*id))
        .chain(times.iter().map(|id| Lane::time(*id)))
        .collect()
}

pub fn sample(lanes: &[Lane], row: &[usize], pins: &[Option<f32>]) -> SignalSample {
    let mut out = SignalSample::NEUTRAL;
    for (l, lane) in lanes.iter().enumerate() {
        let value = match pins.get(l).copied().flatten() {
            Some(value) => value,
            None => lane.samples[row[l].min(lane.samples.len() - 1)],
        };
        if lane.time {
            out.times[lane.id] = value;
        } else {
            out.values[lane.id] = value;
        }
    }
    out
}

pub struct Cover {
    sizes: Vec<usize>,
    pinned: Vec<Option<usize>>,
    single_at: Vec<usize>,
    singles: Vec<bool>,
    blocks: Vec<(usize, usize, usize)>,
    pair_at: Vec<usize>,
    pairs: Vec<bool>,
    single_cursor: usize,
    pair_cursor: usize,
}

impl Cover {
    pub fn new(sizes: &[usize], pinned: &[Option<usize>]) -> Cover {
        let n = sizes.len();
        let mut single_at = Vec::with_capacity(n);
        let mut singles = Vec::new();
        for (l, size) in sizes.iter().enumerate() {
            single_at.push(singles.len());
            let free = pinned[l].is_none();
            singles.extend((0..*size).map(|_| !free));
        }
        let mut blocks = Vec::new();
        let mut pair_at = vec![usize::MAX; n * n];
        let mut pairs = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                let free = pinned[i].is_none() && pinned[j].is_none();
                pair_at[i * n + j] = pairs.len();
                blocks.push((pairs.len(), i, j));
                pairs.extend((0..sizes[i] * sizes[j]).map(|_| !free));
            }
        }
        Cover {
            sizes: sizes.to_vec(),
            pinned: pinned.to_vec(),
            single_at,
            singles,
            blocks,
            pair_at,
            pairs,
            single_cursor: 0,
            pair_cursor: 0,
        }
    }

    fn pair_index(&self, i: usize, a: usize, j: usize, b: usize) -> Option<usize> {
        let (i, a, j, b) = if i < j { (i, a, j, b) } else { (j, b, i, a) };
        if i == j || a >= self.sizes[i] || b >= self.sizes[j] {
            return None;
        }
        let n = self.sizes.len();
        Some(self.pair_at[i * n + j] + a * self.sizes[j] + b)
    }

    pub fn single_covered(&self, l: usize, v: usize) -> bool {
        v >= self.sizes[l] || self.singles[self.single_at[l] + v]
    }

    pub fn pair_covered(&self, i: usize, a: usize, j: usize, b: usize) -> bool {
        self.pair_index(i, a, j, b).is_none_or(|k| self.pairs[k])
    }

    pub fn allows(&self, l: usize, v: usize) -> bool {
        self.pinned[l].is_none_or(|p| p == v)
    }

    pub fn mark(&mut self, row: &[usize]) {
        for (l, v) in row.iter().enumerate() {
            if *v < self.sizes[l] {
                self.singles[self.single_at[l] + v] = true;
            }
            for (m, w) in row.iter().enumerate().skip(l + 1) {
                if let Some(k) = self.pair_index(l, *v, m, *w) {
                    self.pairs[k] = true;
                }
            }
        }
    }

    pub fn first_single(&mut self) -> Option<(usize, usize)> {
        while self.single_cursor < self.singles.len() && self.singles[self.single_cursor] {
            self.single_cursor += 1;
        }
        let k = self.single_cursor;
        if k >= self.singles.len() {
            return None;
        }
        let l = self.single_at.partition_point(|at| *at <= k) - 1;
        Some((l, k - self.single_at[l]))
    }

    pub fn first_pair(&mut self) -> Option<[(usize, usize); 2]> {
        while self.pair_cursor < self.pairs.len() && self.pairs[self.pair_cursor] {
            self.pair_cursor += 1;
        }
        self.pair_from(self.pair_cursor)
    }

    fn pair_from(&self, k: usize) -> Option<[(usize, usize); 2]> {
        if k >= self.pairs.len() {
            return None;
        }
        let block = self.blocks.partition_point(|(at, _, _)| *at <= k) - 1;
        let (at, i, j) = self.blocks[block];
        let local = k - at;
        Some([(i, local / self.sizes[j]), (j, local % self.sizes[j])])
    }

    pub fn row(&self, seeds: &[(usize, usize)], global: Option<&Cover>) -> Vec<usize> {
        let n = self.sizes.len();
        let mut row: Vec<Option<usize>> = self.pinned.clone();
        for (l, v) in seeds {
            if row[*l].is_none() {
                row[*l] = Some(*v);
            }
        }
        let mut order: Vec<usize> = (0..n).filter(|l| row[*l].is_none()).collect();
        order.sort_by_key(|l| (Reverse(self.sizes[*l]), *l));
        for l in order {
            let mut best = (0u64, 0usize);
            for v in 0..self.sizes[l] {
                let mut score = if self.single_covered(l, v) {
                    0
                } else {
                    1 << 40
                };
                for (f, fixed) in row.iter().enumerate() {
                    let Some(w) = *fixed else {
                        continue;
                    };
                    if !self.pair_covered(l, v, f, w) {
                        score += 1 << 20;
                    }
                    if global.is_some_and(|g| !g.pair_covered(l, v, f, w)) {
                        score += 1;
                    }
                }
                if score > best.0 {
                    best = (score, v);
                }
            }
            row[l] = Some(best.1);
        }
        row.into_iter().map(|v| v.unwrap_or(0)).collect()
    }
}

pub fn pairwise(sizes: &[usize], most: usize) -> Vec<Vec<usize>> {
    let pinned = vec![None; sizes.len()];
    let mut cover = Cover::new(sizes, &pinned);
    let mut out = Vec::new();
    while out.len() < most.max(1) {
        let seeds: Vec<(usize, usize)> = if let Some(single) = cover.first_single() {
            vec![single]
        } else if let Some(pair) = cover.first_pair() {
            pair.to_vec()
        } else if out.is_empty() {
            Vec::new()
        } else {
            break;
        };
        let row = cover.row(&seeds, None);
        cover.mark(&row);
        out.push(row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers_every_pair(sizes: &[usize], rows: &[Vec<usize>]) -> bool {
        (0..sizes.len()).all(|i| {
            (i + 1..sizes.len()).all(|j| {
                (0..sizes[i])
                    .all(|a| (0..sizes[j]).all(|b| rows.iter().any(|r| r[i] == a && r[j] == b)))
            })
        })
    }

    #[test]
    fn pairwise_rows_cover_every_value_pair_in_far_fewer_rows_than_every_combination() {
        let sizes = [6, 5, 7, 2, 2, 2, 6, 6, 4];
        let rows = pairwise(&sizes, usize::MAX);
        assert!(covers_every_pair(&sizes, &rows) && rows.len() < 80);
        assert_eq!(rows, pairwise(&sizes, usize::MAX));
        let capped = pairwise(&[6, 5, 7], 7);
        assert!([6, 5, 7]
            .iter()
            .enumerate()
            .all(|(l, size)| (0..*size).all(|v| capped.iter().any(|r| r[l] == v))));
        let sizes = [2, 6, 5];
        let mut cover = Cover::new(&sizes, &[Some(1), None, None]);
        let mut rows = Vec::new();
        while let Some(pair) = cover.first_pair() {
            let row = cover.row(&pair, None);
            cover.mark(&row);
            rows.push(row);
        }
        assert!(rows.iter().all(|r| r[0] == 1));
        assert!(covers_every_pair(
            &sizes[1..],
            &rows.iter().map(|r| r[1..].to_vec()).collect::<Vec<_>>()
        ));
    }
}
