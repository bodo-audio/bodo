//! The routing graph (project.md section 9.7): the mixer drawn as nodes for complex setups. Channels sit on the
//! left, then the inserts in columns by how many routes lead to them, and the master on the right. Lines are routes:
//! solid for audio, dashed for sidechains, faint for an insert feeding the master. Click a node to select its insert.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bodo_model::mixer::{RouteKind, RouteSource};
use bodo_model::{ChannelId, InsertId, Project};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use super::{insert_label, insert_ref, number_of};
use crate::theme::{self, Tokens, size};
use crate::{BodoApp, control};

/// A node's size.
const NODE: egui::Vec2 = vec2(104.0, 28.0);
/// The distance between columns and between rows.
const COLUMN: f32 = 150.0;
const ROW: f32 = 38.0;

/// A node of the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Channel(ChannelId),
    Insert(InsertId),
}

/// A line of the graph.
#[derive(Clone, Copy, Debug)]
struct Line {
    from: Node,
    to: Node,
    kind: Kind,
    level: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Target,
    Audio,
    Sidechain,
    Master,
}

/// Every node with its column, and every line.
///
/// Complexity: O(V + E) in inserts, channels and routes.
fn layout(project: &Project) -> (Vec<(Node, usize)>, Vec<Line>) {
    let master = project.mixer.master.id;
    let mut lines = Vec::new();
    for id in &project.rack_order {
        if let Some(channel) = project.channels.get(id) {
            let to = channel.target.filter(|t| project.mixer.insert(*t).is_some()).unwrap_or(master);
            lines.push(Line { from: Node::Channel(*id), to: Node::Insert(to), kind: Kind::Target, level: channel.volume });
        }
    }
    for (key, route) in project.mixer.routes.iter() {
        let from = match key.from {
            RouteSource::Channel(c) => Node::Channel(c),
            RouteSource::Insert(i) => Node::Insert(i),
        };
        let kind = if route.kind == RouteKind::Sidechain { Kind::Sidechain } else { Kind::Audio };
        lines.push(Line { from, to: Node::Insert(key.to), kind, level: route.send_level });
    }
    for id in project.mixer.order.iter() {
        if project.mixer.inserts.get(id).is_some_and(|i| i.to_master) {
            lines.push(Line { from: Node::Insert(*id), to: Node::Insert(master), kind: Kind::Master, level: 1.0 });
        }
    }
    // Columns: channels at 0; each insert one past the deepest insert routing into it (routes never loop).
    let mut incoming: HashMap<InsertId, usize> = HashMap::new();
    let mut out: HashMap<InsertId, Vec<InsertId>> = HashMap::new();
    for line in &lines {
        if let (Node::Insert(a), Node::Insert(b)) = (line.from, line.to)
            && b != master
        {
            *incoming.entry(b).or_default() += 1;
            out.entry(a).or_default().push(b);
        }
    }
    let mut depth: HashMap<InsertId, usize> = HashMap::new();
    let mut queue: VecDeque<InsertId> = project.mixer.order.iter().copied().filter(|id| !incoming.contains_key(id)).collect();
    for id in &queue {
        depth.insert(*id, 1);
    }
    while let Some(id) = queue.pop_front() {
        let d = depth.get(&id).copied().unwrap_or(1);
        for next in out.get(&id).cloned().unwrap_or_default() {
            let entry = depth.entry(next).or_insert(1);
            *entry = (*entry).max(d + 1);
            if let Some(count) = incoming.get_mut(&next) {
                *count -= 1;
                if *count == 0 {
                    queue.push_back(next);
                }
            }
        }
    }
    let deepest = depth.values().copied().max().unwrap_or(1);
    let mut nodes: Vec<(Node, usize)> = project.rack_order.iter().map(|id| (Node::Channel(*id), 0)).collect();
    nodes.extend(project.mixer.order.iter().map(|id| (Node::Insert(*id), depth.get(id).copied().unwrap_or(1))));
    nodes.push((Node::Insert(master), deepest + 1));
    (nodes, lines)
}

/// Draws the routing graph.
pub(super) fn show(app: &mut BodoApp, ui: &mut Ui, project: &Arc<Project>) {
    let tk = Tokens::get();
    let (nodes, lines) = layout(project);
    let columns = nodes.iter().map(|(_, c)| *c).max().unwrap_or(0) + 1;
    let mut rows = vec![0usize; columns];
    let mut places: HashMap<Node, Rect> = HashMap::new();
    for (node, column) in &nodes {
        let row = rows[*column];
        rows[*column] += 1;
        places.insert(*node, Rect::from_min_size(pos2(f32::from(*column as u16) * COLUMN, row as f32 * ROW), NODE));
    }
    let tallest = rows.iter().copied().max().unwrap_or(1) as f32;
    let canvas = vec2(columns as f32 * COLUMN + 20.0, tallest * ROW + 20.0);
    egui::ScrollArea::both().id_salt("bodo-mixer-graph").auto_shrink([false, false]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(canvas.max(ui.available_size()), Sense::hover());
        let origin = area.min.to_vec2() + vec2(10.0, 10.0);
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        painter.rect_filled(area, 0.0, tk.panel);
        let place = |node: &Node| places.get(node).map(|r| r.translate(origin));
        for line in &lines {
            let (Some(a), Some(b)) = (place(&line.from), place(&line.to)) else { continue };
            let color = match line.kind {
                Kind::Target => tk.text_faint,
                Kind::Audio => tk.accent,
                Kind::Sidechain => tk.warn,
                Kind::Master => tk.text_dim,
            };
            let width = if line.kind == Kind::Audio { 1.0 + line.level.clamp(0.0, 2.0) } else { 1.2 };
            edge(&painter, a.right_center(), b.left_center(), Stroke::new(width, color), line.kind == Kind::Sidechain);
        }
        let selected = app.session.editor().insert;
        for (node, _) in &nodes {
            let Some(rect) = place(node) else { continue };
            let (name, color, id) = match node {
                Node::Channel(c) => {
                    let Some(channel) = project.channels.get(c) else { continue };
                    (channel.name.clone(), theme::color(channel.color), None)
                }
                Node::Insert(i) => {
                    let number = number_of(project, *i).unwrap_or(0);
                    let Some(insert) = project.mixer.insert(*i) else { continue };
                    let title = if number == 0 { String::from("Master") } else { format!("{number} {}", insert.name) };
                    (title, theme::color(insert.color), Some((*i, number)))
                }
            };
            let response = ui.interact(rect, ui.id().with(("graph-node", format!("{node:?}"))), Sense::click());
            let chosen = id.is_some_and(|(i, _)| selected == Some(i));
            let fill = if chosen {
                tk.accent_soft
            } else if response.hovered() {
                tk.control_hover
            } else {
                tk.chrome
            };
            painter.rect(rect, 4.0, fill, Stroke::new(1.0, if chosen { tk.accent } else { tk.border }), StrokeKind::Inside);
            painter.rect_filled(Rect::from_min_size(rect.min + vec2(3.0, 4.0), vec2(3.0, rect.height() - 8.0)), 1.0, color);
            painter.with_clip_rect(rect.shrink(2.0)).text(
                pos2(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(size::SMALL),
                tk.text,
            );
            match id {
                Some((insert, number)) => {
                    control::record(ui, &response, &format!("Graph {}", insert_label(number)), "node", Some(chosen));
                    app.hint(&response, format!("{name}: click to select it"));
                    if response.clicked() {
                        let _ = app.run("edit.select", json!({ "insert": insert_ref(number) }));
                    }
                    let _ = insert;
                }
                None => {
                    control::record(ui, &response, &format!("Graph channel {name}"), "node", None);
                    app.hint(&response, format!("The channel {name}"));
                }
            }
        }
    });
}

/// A curve from `a` to `b`, dashed for sidechains.
fn edge(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, dashed: bool) {
    let bend = ((b.x - a.x).abs() * 0.5).max(30.0);
    let curve =
        egui::epaint::CubicBezierShape::from_points_stroke([a, a + vec2(bend, 0.0), b - vec2(bend, 0.0), b], false, Color32::TRANSPARENT, stroke);
    if dashed {
        let points: Vec<Pos2> = curve.flatten(Some(0.5));
        painter.extend(Shape::dashed_line(&points, stroke, 5.0, 4.0));
    } else {
        painter.add(curve);
    }
    // An arrow head where it arrives.
    painter.add(Shape::convex_polygon(vec![b, b + vec2(-6.0, -3.5), b + vec2(-6.0, 3.5)], stroke.color, Stroke::NONE));
}
