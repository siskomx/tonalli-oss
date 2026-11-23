use super::{corrupt, limit, number, triangle, MAX_BYTES, MAX_TRIANGLES, MAX_VERTICES};
use crate::{
    documents::Archive,
    pack::{PackError, PackSource},
};
use quick_xml::{
    encoding::DecodingReader, events::Event, name::ResolveResult, reader::NsReader, XmlVersion,
};
use std::{collections::HashMap, sync::Arc};
const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
type Attributes = HashMap<String, String>;
type Matrix = [f64; 12];
const IDENTITY: Matrix = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];
#[derive(Default)]
struct Object {
    vertices: Vec<[f64; 3]>,
    faces: Vec<[usize; 3]>,
    components: Vec<(u32, Matrix)>,
    mesh: bool,
    group: bool,
    seen_vertices: bool,
    seen_faces: bool,
}

// Stream XML; unknown extension subtrees are inert and never become core elements.
fn walk(
    bytes: &[u8],
    namespace: &str,
    root: &str,
    mut visit: impl FnMut(&[String], Option<&Attributes>) -> Result<(), PackError>,
) -> Result<(), PackError> {
    let mut reader = NsReader::from_reader(DecodingReader::new(bytes));
    let mut buffer = Vec::new();
    let mut path = Vec::new();
    let mut roots = 0;
    let mut count = 0;
    let mut version = XmlVersion::Implicit1_0;
    loop {
        count += 1;
        if count > 1_000_000 {
            return Err(limit());
        }
        let event = reader.read_event_into(&mut buffer).map_err(|_| corrupt())?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Decl(declaration) => {
                version = declaration.xml_version().map_err(|_| corrupt())?;
                if declaration.encoding().is_some() {
                    let encoding = declaration.encoder().ok_or_else(corrupt)?;
                    reader.get_mut().set_encoding(encoding);
                }
            }
            Event::DocType(_) => return Err(corrupt()),
            Event::Start(element) | Event::Empty(element) => {
                let supported = matches!(reader.resolver().resolve_element(element.name()).0,ResolveResult::Bound(value) if value.as_ref()==namespace);
                let name = if supported {
                    element.local_name().as_ref().to_string()
                } else {
                    String::new()
                };
                if path.is_empty() {
                    roots += 1;
                    if roots != 1 || name != root {
                        return Err(corrupt());
                    }
                }
                if path.len() >= 64 {
                    return Err(limit());
                }
                let mut attrs = HashMap::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|_| corrupt())?;
                    let key = attribute.key.as_ref().to_string();
                    let value = attribute
                        .normalized_value(version)
                        .map_err(|_| corrupt())?
                        .into_owned();
                    if attrs.insert(key, value).is_some() {
                        return Err(corrupt());
                    }
                }
                path.push(name);
                visit(&path, Some(&attrs))?;
                if empty {
                    visit(&path, None)?;
                    path.pop();
                }
            }
            Event::End(_) => {
                if path.is_empty() {
                    return Err(corrupt());
                }
                visit(&path, None)?;
                path.pop();
            }
            Event::Text(text) if path.is_empty() && !text.as_ref().trim().is_empty() => {
                return Err(corrupt())
            }
            Event::Eof => {
                if !path.is_empty() || roots != 1 {
                    return Err(corrupt());
                }
                return Ok(());
            }
            _ => {}
        }
        buffer.clear();
    }
}
fn required<'a>(attrs: &'a Attributes, key: &str) -> Result<&'a str, PackError> {
    attrs.get(key).map(String::as_str).ok_or_else(corrupt)
}
fn id(attrs: &Attributes, key: &str) -> Result<u32, PackError> {
    let value = required(attrs, key)?
        .parse::<u32>()
        .map_err(|_| corrupt())?;
    if value == 0 {
        return Err(corrupt());
    }
    Ok(value)
}
fn matrix(attrs: &Attributes) -> Result<Matrix, PackError> {
    if attrs
        .keys()
        .any(|key| key.ends_with(":path") || key == "path")
    {
        return Err(limit());
    }
    let Some(text) = attrs.get("transform") else {
        return Ok(IDENTITY);
    };
    let values = text
        .split_whitespace()
        .map(number)
        .collect::<Result<Vec<_>, _>>()?;
    values.try_into().map_err(|_| corrupt())
}
fn target(value: &str) -> Result<String, PackError> {
    let base = url::Url::parse("https://model.invalid/").map_err(|_| corrupt())?;
    let url = base.join(value).map_err(|_| corrupt())?;
    if url.origin() != base.origin()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(corrupt());
    }
    let decoded = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| corrupt())?;
    if decoded.split('/').any(|part| part == ".." || part == ".")
        || decoded.contains(['\\', ':', '\0'])
        || decoded.chars().any(char::is_control)
    {
        return Err(corrupt());
    }
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|_| corrupt())?;
    let path = path.strip_prefix('/').ok_or_else(corrupt)?;
    if path.is_empty() || path.split('/').any(str::is_empty) {
        return Err(corrupt());
    }
    Ok(path.to_owned())
}
pub(super) fn parse(source: Arc<dyn PackSource>) -> Result<Vec<f32>, PackError> {
    let archive = Archive::open(source)?;
    let relationships = archive.read("_rels/.rels", 1024 * 1024)?;
    let mut part = None;
    walk(&relationships, REL, "Relationships", |path, attrs| {
        if path
            .iter()
            .map(String::as_str)
            .eq(["Relationships", "Relationship"])
        {
            if let Some(attrs) = attrs {
                if attrs.get("Type").map(String::as_str)
                    == Some("http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel")
                {
                    if part.is_some()
                        || attrs
                            .get("TargetMode")
                            .is_some_and(|mode| mode != "Internal")
                    {
                        return Err(corrupt());
                    }
                    part = Some(target(required(attrs, "Target")?)?);
                }
            }
        }
        Ok(())
    })?;
    let bytes = archive.read(&part.ok_or_else(corrupt)?, MAX_BYTES)?;
    let mut objects = HashMap::<u32, Object>::new();
    let mut current = None;
    let mut build = Vec::new();
    let mut vertices = 0usize;
    let mut faces = 0usize;
    let mut references = 0usize;
    let mut unit = 1.;
    walk(&bytes, CORE, "model", |path, attrs| {
        let names: Vec<_> = path.iter().map(String::as_str).collect();
        let Some(attrs) = attrs else {
            if names == ["model", "resources", "object"] {
                current = None;
            }
            return Ok(());
        };
        match names.as_slice() {
            ["model"] => {
                if attrs
                    .get("requiredextensions")
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return Err(limit());
                }
                unit = match attrs
                    .get("unit")
                    .map(String::as_str)
                    .unwrap_or("millimeter")
                {
                    "micron" => 0.001,
                    "millimeter" => 1.,
                    "centimeter" => 10.,
                    "inch" => 25.4,
                    "foot" => 304.8,
                    "meter" => 1000.,
                    _ => return Err(limit()),
                };
            }
            ["model", "resources", "object"] => {
                if objects.len() >= 4096 {
                    return Err(limit());
                }
                let value = id(attrs, "id")?;
                if objects.insert(value, Object::default()).is_some() {
                    return Err(corrupt());
                }
                current = Some(value);
            }
            ["model", "resources", "object", "mesh"] => {
                let object = objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?;
                if object.mesh || object.group {
                    return Err(corrupt());
                }
                object.mesh = true;
            }
            ["model", "resources", "object", "components"] => {
                let object = objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?;
                if object.mesh || object.group {
                    return Err(corrupt());
                }
                object.group = true;
            }
            ["model", "resources", "object", "mesh", "vertices"] => {
                let object = objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?;
                if object.seen_vertices {
                    return Err(corrupt());
                }
                object.seen_vertices = true;
            }
            ["model", "resources", "object", "mesh", "triangles"] => {
                let object = objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?;
                if object.seen_faces {
                    return Err(corrupt());
                }
                object.seen_faces = true;
            }
            ["model", "resources", "object", "mesh", "vertices", "vertex"] => {
                vertices += 1;
                if vertices > MAX_VERTICES {
                    return Err(limit());
                }
                let point = [
                    number(required(attrs, "x")?)?,
                    number(required(attrs, "y")?)?,
                    number(required(attrs, "z")?)?,
                ];
                objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?
                    .vertices
                    .push(point);
            }
            ["model", "resources", "object", "mesh", "triangles", "triangle"] => {
                faces += 1;
                if faces > MAX_TRIANGLES {
                    return Err(limit());
                }
                let mut face = [0; 3];
                for (index, key) in ["v1", "v2", "v3"].into_iter().enumerate() {
                    face[index] = required(attrs, key)?
                        .parse::<usize>()
                        .map_err(|_| corrupt())?;
                }
                objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?
                    .faces
                    .push(face);
            }
            ["model", "resources", "object", "components", "component"] => {
                references += 1;
                if references > 10000 {
                    return Err(limit());
                }
                let component = (id(attrs, "objectid")?, matrix(attrs)?);
                objects
                    .get_mut(&current.ok_or_else(corrupt)?)
                    .ok_or_else(corrupt)?
                    .components
                    .push(component);
            }
            ["model", "build", "item"] => {
                if build.len() >= 4096 {
                    return Err(limit());
                }
                build.push((id(attrs, "objectid")?, matrix(attrs)?));
            }
            _ => {}
        }
        Ok(())
    })?;
    let mut output = Vec::new();
    let mut ancestors = Vec::new();
    let mut transforms = Vec::new();
    let mut work = 0;
    for (object, transform) in build {
        transforms.push(transform);
        emit(
            object,
            &objects,
            &mut ancestors,
            &mut transforms,
            unit,
            &mut work,
            &mut output,
        )?;
        transforms.pop();
    }
    Ok(output)
}
fn emit(
    id: u32,
    objects: &HashMap<u32, Object>,
    ancestors: &mut Vec<u32>,
    transforms: &mut Vec<Matrix>,
    unit: f64,
    work: &mut usize,
    output: &mut Vec<f32>,
) -> Result<(), PackError> {
    *work += 1;
    if *work > 10000 || ancestors.len() >= 32 {
        return Err(limit());
    }
    if ancestors.contains(&id) {
        return Err(corrupt());
    }
    let object = objects.get(&id).ok_or_else(corrupt)?;
    if !object.mesh && !object.group {
        return Err(corrupt());
    }
    ancestors.push(id);
    if object.mesh {
        if object.vertices.is_empty() || object.faces.is_empty() {
            return Err(corrupt());
        }
        for face in &object.faces {
            let mut points = [[0.; 3]; 3];
            for (index, vertex) in face.iter().enumerate() {
                let mut point = *object.vertices.get(*vertex).ok_or_else(corrupt)?;
                for m in transforms.iter().rev() {
                    point = [
                        point[0] * m[0] + point[1] * m[3] + point[2] * m[6] + m[9],
                        point[0] * m[1] + point[1] * m[4] + point[2] * m[7] + m[10],
                        point[0] * m[2] + point[1] * m[5] + point[2] * m[8] + m[11],
                    ];
                    if point
                        .iter()
                        .any(|value| !value.is_finite() || value.abs() > 1e12)
                    {
                        return Err(corrupt());
                    }
                }
                for value in &mut point {
                    *value *= unit;
                }
                points[index] = point;
            }
            triangle(output, points)?;
        }
    } else {
        if object.components.is_empty() {
            return Err(corrupt());
        }
        for (component, transform) in &object.components {
            transforms.push(*transform);
            emit(
                *component, objects, ancestors, transforms, unit, work, output,
            )?;
            transforms.pop();
        }
    }
    ancestors.pop();
    Ok(())
}
