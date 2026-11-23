//! Bounded geometry previews from immutable positioned managed sources.
mod three_mf;
use crate::pack::{PackError, PackSource};
use serde::Serialize;
use std::sync::Arc;
pub enum ModelFormat {
    Stl,
    Obj,
    ThreeMf,
}
#[derive(Debug, Serialize)]
pub struct Model {
    pub positions: Vec<f32>,
}
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRIANGLES: usize = 100_000;
const MAX_VERTICES: usize = 300_000;
fn corrupt() -> PackError {
    PackError::Corrupt("Invalid model geometry".into())
}
fn limit() -> PackError {
    PackError::Mismatch("This model exceeds preview limits or uses unsupported geometry".into())
}
fn number(value: &str) -> Result<f64, PackError> {
    let result = value.parse::<f64>().map_err(|_| corrupt())?;
    if !result.is_finite() || result.abs() > 1e12 {
        return Err(corrupt());
    }
    Ok(result)
}
fn read(source: &dyn PackSource) -> Result<Vec<u8>, PackError> {
    let length = source.len();
    if length > MAX_BYTES {
        return Err(limit());
    }
    let mut bytes = vec![0; length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        let n = source.read_at(offset as u64, &mut bytes[offset..])?;
        if n == 0 {
            return Err(PackError::Truncated);
        }
        if n > bytes.len() - offset {
            return Err(corrupt());
        }
        offset += n;
    }
    Ok(bytes)
}
fn triangle(output: &mut Vec<f32>, points: [[f64; 3]; 3]) -> Result<(), PackError> {
    if output.len() / 9 >= MAX_TRIANGLES {
        return Err(limit());
    }
    for point in points {
        for value in point {
            if !value.is_finite() || value.abs() > 1e12 {
                return Err(corrupt());
            }
            output.push(value as f32);
        }
    }
    Ok(())
}
pub fn load(source: Arc<dyn PackSource>, format: ModelFormat) -> Result<Model, PackError> {
    if source.len() == 0 || source.len() > MAX_BYTES {
        return Err(limit());
    }
    let positions = match format {
        ModelFormat::Stl => stl(&read(source.as_ref())?)?,
        ModelFormat::Obj => obj(&read(source.as_ref())?)?,
        ModelFormat::ThreeMf => three_mf::parse(source)?,
    };
    if positions.is_empty() {
        return Err(corrupt());
    }
    Ok(Model { positions })
}
fn stl(bytes: &[u8]) -> Result<Vec<f32>, PackError> {
    let mut output = Vec::new();
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes(bytes[80..84].try_into().map_err(|_| corrupt())?) as usize;
        if count <= MAX_TRIANGLES && 84 + 50 * count == bytes.len() {
            for facet in bytes[84..].chunks_exact(50) {
                let mut points = [[0.; 3]; 3];
                for (vertex, point) in points.iter_mut().enumerate() {
                    for (axis, value) in point.iter_mut().enumerate() {
                        let offset = 12 + vertex * 12 + axis * 4;
                        *value = f32::from_le_bytes(
                            facet[offset..offset + 4]
                                .try_into()
                                .map_err(|_| corrupt())?,
                        ) as f64;
                    }
                }
                triangle(&mut output, points)?;
            }
            return Ok(output);
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|_| corrupt())?;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next().and_then(|line| line.split_whitespace().next()) != Some("solid") {
        return Err(corrupt());
    }
    let mut ended = false;
    while let Some(line) = lines.next() {
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("endsolid") => {
                ended = true;
                break;
            }
            Some("facet") => {
                if tokens.next() != Some("normal") {
                    return Err(corrupt());
                }
                for _ in 0..3 {
                    number(tokens.next().ok_or_else(corrupt)?)?;
                }
                if tokens.next().is_some() || lines.next() != Some("outer loop") {
                    return Err(corrupt());
                }
                let mut points = [[0.; 3]; 3];
                for point in &mut points {
                    let mut row = lines.next().ok_or_else(corrupt)?.split_whitespace();
                    if row.next() != Some("vertex") {
                        return Err(corrupt());
                    }
                    for value in point {
                        *value = number(row.next().ok_or_else(corrupt)?)?;
                    }
                    if row.next().is_some() {
                        return Err(corrupt());
                    }
                }
                if lines.next() != Some("endloop") || lines.next() != Some("endfacet") {
                    return Err(corrupt());
                }
                triangle(&mut output, points)?;
            }
            _ => return Err(corrupt()),
        }
    }
    if !ended || lines.next().is_some() {
        return Err(corrupt());
    }
    Ok(output)
}
fn obj(bytes: &[u8]) -> Result<Vec<f32>, PackError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| corrupt())?
        .replace("\\\r\n", " ")
        .replace("\\\n", " ");
    let mut vertices = Vec::new();
    let mut output = Vec::new();
    let mut face_work = 0usize;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        if line.len() > 65536 {
            return Err(limit());
        }
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("v") => {
                if vertices.len() >= MAX_VERTICES {
                    return Err(limit());
                }
                let mut point = [0.; 3];
                for value in &mut point {
                    *value = number(tokens.next().ok_or_else(corrupt)?)?;
                }
                let extra = tokens.map(number).collect::<Result<Vec<_>, _>>()?;
                if extra.len() == 1 {
                    if extra[0] == 0. {
                        return Err(corrupt());
                    }
                    for value in &mut point {
                        *value /= extra[0];
                    }
                } else if !extra.is_empty() && extra.len() != 3 {
                    return Err(limit());
                }
                vertices.push(point);
            }
            Some("f") => {
                let mut face = Vec::new();
                for token in tokens {
                    if face.len() >= 256 {
                        return Err(limit());
                    }
                    let index = token
                        .split('/')
                        .next()
                        .ok_or_else(corrupt)?
                        .parse::<i64>()
                        .map_err(|_| corrupt())?;
                    let offset = if index > 0 {
                        index - 1
                    } else {
                        vertices.len() as i64 + index
                    };
                    if index == 0 || offset < 0 {
                        return Err(corrupt());
                    }
                    face.push(*vertices.get(offset as usize).ok_or_else(corrupt)?);
                }
                face_work = face_work
                    .checked_add(face.len() * face.len())
                    .ok_or_else(limit)?;
                if face_work > 20_000_000 {
                    return Err(limit());
                }
                polygon(&face, &mut output)?;
            }
            Some("l" | "p" | "curv" | "surf" | "cstype") => return Err(limit()),
            // Names, normals, texture coordinates and material references never cause I/O.
            Some("o" | "g" | "s" | "usemtl" | "mtllib" | "vt" | "vn") | None => {}
            Some(_) => return Err(limit()),
        }
    }
    Ok(output)
}
// Ear clipping preserves concave polygon outlines instead of silently filling their notches.
fn polygon(points: &[[f64; 3]], output: &mut Vec<f32>) -> Result<(), PackError> {
    if points.len() < 3 {
        return Err(corrupt());
    }
    if points.len() == 3 {
        return triangle(output, [points[0], points[1], points[2]]);
    }
    let mut normal = [0.; 3];
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let drop = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .ok_or_else(corrupt)?;
    let axes: Vec<_> = (0..3).filter(|axis| *axis != drop).collect();
    let projected: Vec<_> = points.iter().map(|p| [p[axes[0]], p[axes[1]]]).collect();
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let area: f64 = (0..points.len())
        .map(|i| {
            let a = projected[i];
            let b = projected[(i + 1) % points.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area == 0. || !area.is_finite() {
        return Err(corrupt());
    }
    let sign = area.signum();
    let mut ring: Vec<_> = (0..points.len()).collect();
    while ring.len() > 3 {
        let mut ear = None;
        for i in 0..ring.len() {
            let a = ring[(i + ring.len() - 1) % ring.len()];
            let b = ring[i];
            let c = ring[(i + 1) % ring.len()];
            if cross(projected[a], projected[b], projected[c]) * sign <= 0. {
                continue;
            }
            if ring
                .iter()
                .copied()
                .filter(|p| *p != a && *p != b && *p != c)
                .any(|p| {
                    cross(projected[a], projected[b], projected[p]) * sign >= 0.
                        && cross(projected[b], projected[c], projected[p]) * sign >= 0.
                        && cross(projected[c], projected[a], projected[p]) * sign >= 0.
                })
            {
                continue;
            }
            ear = Some((i, a, b, c));
            break;
        }
        let (i, a, b, c) = ear.ok_or_else(corrupt)?;
        triangle(output, [points[a], points[b], points[c]])?;
        ring.remove(i);
    }
    triangle(output, [points[ring[0]], points[ring[1]], points[ring[2]]])
}
