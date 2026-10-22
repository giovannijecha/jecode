//! Journal encoding of records.
use super::*;

impl Record {
    pub fn frame(&self) -> Vec<u8> {
        let mut out = Encoder::default();
        let kind = match self {
            Self::Recovery(recovery) => {
                match recovery {
                    Recovery::Mode { seq, time } => {
                        out.u8(1).u64(*seq).u64(*time);
                    }
                    Recovery::Snapshot {
                        seq,
                        group,
                        snapshot,
                    } => {
                        out.u8(2).u64(*seq).u64(*group).u64(*snapshot);
                    }
                    Recovery::Step {
                        seq,
                        group,
                        snapshot,
                        path,
                        before,
                    } => {
                        out.u8(3).u64(*seq).u64(*group).u64(*snapshot).text(path);
                        before.encode(&mut out);
                    }
                    Recovery::Scope {
                        seq,
                        group,
                        snapshot,
                        root,
                    } => {
                        out.u8(4).u64(*seq).u64(*group).u64(*snapshot).text(root);
                    }
                    Recovery::Status {
                        seq,
                        group,
                        action,
                        time,
                        until,
                    } => {
                        let action = match action {
                            RecoveryAction::Accepted => 1,
                            RecoveryAction::Touched => 4,
                        };
                        out.u8(5)
                            .u64(*seq)
                            .u64(*group)
                            .u8(action)
                            .u64(*time)
                            .u64(*until);
                    }
                }
                Kind::Recovery
            }
            Self::Version(version) => {
                out.u64(version.seq).text(&version.path);
                version.state.encode(&mut out);
                match &version.stat {
                    Some(stat) => stat.encode(out.u8(1)),
                    None => {
                        out.u8(0);
                    }
                }
                out.u64(version.time)
                    .text(&version.origin)
                    .u64(version.operation)
                    .u8(version.discovery as u8);
                Kind::Version
            }
            Self::Stat { seq, path, stat } => {
                out.u64(*seq).text(path);
                stat.encode(&mut out);
                Kind::Stat
            }
            Self::Prepare(prepare) => {
                out.u64(prepare.seq)
                    .text(&prepare.owner)
                    .text(&prepare.origin)
                    .text(&prepare.kind)
                    .text(&prepare.path);
                prepare.before.encode(&mut out);
                prepare.after.encode(&mut out);
                out.u64(prepare.time);
                Kind::Prepare
            }
            Self::Complete {
                seq,
                operation,
                outcome,
            } => {
                out.u64(*seq).u64(*operation).u8(*outcome as u8);
                Kind::Complete
            }
            Self::Point(point) => {
                out.u64(point.seq)
                    .text(&point.kind)
                    .text(&point.label)
                    .text(&point.origin)
                    .u64(point.time)
                    .u64(point.policy)
                    .u8(u8::from(point.pinned));
                Kind::Point
            }
            Self::Pin {
                seq,
                target,
                pinned,
                label,
            } => {
                out.u64(*seq).u64(*target).u8(u8::from(*pinned)).text(label);
                Kind::Pin
            }
            Self::Policy { seq, id, text } => {
                out.u64(*seq).u64(*id).text(text);
                Kind::Policy
            }
            Self::Event(event) => {
                out.u64(event.seq)
                    .text(&event.origin)
                    .u64(event.time)
                    .text(&event.text)
                    .u64(event.paths.len() as u64);
                for path in &event.paths {
                    out.text(path);
                }
                Kind::Event
            }
            Self::Coverage {
                seq,
                complete,
                reason,
            } => {
                out.u64(*seq).u8(u8::from(*complete)).text(reason);
                Kind::Coverage
            }
            Self::Moved { seq, generation } => {
                out.u64(*seq).u64(*generation);
                Kind::Moved
            }
        };
        encode(kind, &out.0)
    }

    pub fn decode(kind: Kind, payload: &[u8]) -> Result<Self, String> {
        let mut input = Decoder::new(payload);
        let flag = |value: u8| value != 0;
        Ok(match kind {
            Kind::Object => return Err("object frames are decoded separately".into()),
            Kind::Recovery => Self::Recovery(match input.u8()? {
                1 => Recovery::Mode {
                    seq: input.u64()?,
                    time: input.u64()?,
                },
                2 => Recovery::Snapshot {
                    seq: input.u64()?,
                    group: input.u64()?,
                    snapshot: input.u64()?,
                },
                3 => Recovery::Step {
                    seq: input.u64()?,
                    group: input.u64()?,
                    snapshot: input.u64()?,
                    path: input.text()?,
                    before: State::decode(&mut input)?,
                },
                4 => Recovery::Scope {
                    seq: input.u64()?,
                    group: input.u64()?,
                    snapshot: input.u64()?,
                    root: input.text()?,
                },
                5 => {
                    let seq = input.u64()?;
                    let group = input.u64()?;
                    let action = match input.u8()? {
                        1 => RecoveryAction::Accepted,
                        4 => RecoveryAction::Touched,
                        _ => return Err("unknown recovery action".into()),
                    };
                    Recovery::Status {
                        seq,
                        group,
                        action,
                        time: input.u64()?,
                        until: input.u64()?,
                    }
                }
                _ => return Err("unknown recovery record".into()),
            }),
            Kind::Version => Self::Version(Version {
                seq: input.u64()?,
                path: input.text()?,
                state: State::decode(&mut input)?,
                stat: if flag(input.u8()?) {
                    Some(Stat::decode(&mut input)?)
                } else {
                    None
                },
                time: input.u64()?,
                origin: input.text()?,
                operation: input.u64()?,
                discovery: match input.u8()? {
                    1 => Discovery::Created,
                    2 => Discovery::Found,
                    _ => Discovery::Known,
                },
            }),
            Kind::Stat => Self::Stat {
                seq: input.u64()?,
                path: input.text()?,
                stat: Stat::decode(&mut input)?,
            },
            Kind::Prepare => Self::Prepare(Prepare {
                seq: input.u64()?,
                owner: input.text()?,
                origin: input.text()?,
                kind: input.text()?,
                path: input.text()?,
                before: State::decode(&mut input)?,
                after: State::decode(&mut input)?,
                time: input.u64()?,
            }),
            Kind::Complete => Self::Complete {
                seq: input.u64()?,
                operation: input.u64()?,
                outcome: match input.u8()? {
                    1 => Outcome::Applied,
                    2 => Outcome::NotApplied,
                    _ => Outcome::Unknown,
                },
            },
            Kind::Point => Self::Point(Point {
                seq: input.u64()?,
                kind: input.text()?,
                label: input.text()?,
                origin: input.text()?,
                time: input.u64()?,
                policy: input.u64()?,
                pinned: flag(input.u8()?),
            }),
            Kind::Pin => Self::Pin {
                seq: input.u64()?,
                target: input.u64()?,
                pinned: flag(input.u8()?),
                label: input.text()?,
            },
            Kind::Policy => Self::Policy {
                seq: input.u64()?,
                id: input.u64()?,
                text: input.text()?,
            },
            Kind::Event => {
                let seq = input.u64()?;
                let origin = input.text()?;
                let time = input.u64()?;
                let text = input.text()?;
                let count = input.u64()?;
                let mut paths = Vec::new();
                for _ in 0..count.min(4096) {
                    paths.push(input.text()?);
                }
                Self::Event(Event {
                    seq,
                    origin,
                    time,
                    text,
                    paths,
                })
            }
            Kind::Coverage => Self::Coverage {
                seq: input.u64()?,
                complete: flag(input.u8()?),
                reason: input.text()?,
            },
            Kind::Moved => Self::Moved {
                seq: input.u64()?,
                generation: input.u64()?,
            },
        })
    }
}
