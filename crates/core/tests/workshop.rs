use std::{
    io::{Cursor, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tonalli_core::{
    pack::{PackError, PackSource},
    workshop::{load, ModelFormat},
};
use zip::{write::SimpleFileOptions, ZipWriter};
struct Source {
    bytes: Vec<u8>,
    removed: AtomicBool,
}
impl PackSource for Source {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<usize, PackError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(PackError::MediaRemoved);
        }
        let start = usize::try_from(offset).map_err(|_| PackError::Truncated)?;
        let bytes = self.bytes.get(start..).unwrap_or(&[]);
        let n = bytes.len().min(out.len()).min(17);
        out[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
}
fn source(bytes: impl Into<Vec<u8>>) -> Arc<Source> {
    Arc::new(Source {
        bytes: bytes.into(),
        removed: AtomicBool::new(false),
    })
}
fn archive(xml: &str, target: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="{target}"/></Relationships>"#
            ),
        ),
        ("3D/main.model", xml.into()),
    ] {
        zip.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(bytes.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
const OBJECT: &str = r#"<object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#;
fn model(resources: &str, build: &str) -> String {
    format!(
        r#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" unit="millimeter"><resources>{resources}</resources><build>{build}</build></model>"#
    )
}
#[test]
fn binary_and_ascii_stl_are_positioned_and_bounded() {
    let mut binary = vec![0; 84];
    binary[..5].copy_from_slice(b"solid");
    binary[80..84].copy_from_slice(&1u32.to_le_bytes());
    binary.extend_from_slice(&[0; 12]);
    for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    binary.extend_from_slice(&[0; 2]);
    let first = load(source(binary.clone()), ModelFormat::Stl).unwrap();
    assert_eq!(first.positions, vec![0., 0., 0., 1., 0., 0., 0., 1., 0.]);
    let ascii=b"solid test\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid test\n";
    assert_eq!(
        load(source(ascii.to_vec()), ModelFormat::Stl)
            .unwrap()
            .positions,
        first.positions
    );
    binary[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(load(source(binary), ModelFormat::Stl).is_err());
}
#[test]
fn obj_polygons_negative_indices_and_material_references_stay_local() {
    let obj=b"mtllib https://example.invalid/never-fetch.mtl\nv 0 0 0\nv 2 0 0\nv 2 2 0\nv 1 1 0\nv 0 2 0\nf -5/1 -4/2 -3/3 -2/4 -1/5\n";
    let result = load(source(obj.to_vec()), ModelFormat::Obj).unwrap();
    assert_eq!(result.positions.len(), 27);
    let area: f32 = result
        .positions
        .chunks_exact(9)
        .map(|p| ((p[3] - p[0]) * (p[7] - p[1]) - (p[6] - p[0]) * (p[4] - p[1])).abs() / 2.)
        .sum();
    assert!((area - 3.).abs() < 0.0001);
    for text in ["v NaN 0 0\nf 1 1 1", "v 0 0 0\nf 1 2 3", "v 0 0 0\nf 0 1 1"] {
        assert!(load(source(text.as_bytes().to_vec()), ModelFormat::Obj).is_err());
    }
}
#[test]
fn three_mf_build_components_transform_geometry() {
    let resources = format!(
        r#"{OBJECT}<object id="2"><components><component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/></components></object>"#
    );
    let xml = model(
        &resources,
        r#"<item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 20 0"/>"#,
    );
    let result = load(
        source(archive(&xml, "/3D/main.model")),
        ModelFormat::ThreeMf,
    )
    .unwrap();
    assert_eq!(
        result.positions,
        vec![10., 20., 0., 11., 20., 0., 10., 21., 0.]
    );
}
#[test]
fn three_mf_refuses_cycles_external_targets_and_expansion() {
    let cycle = model(
        r#"<object id="1"><components><component objectid="1"/></components></object>"#,
        r#"<item objectid="1"/>"#,
    );
    assert!(load(
        source(archive(&cycle, "/3D/main.model")),
        ModelFormat::ThreeMf
    )
    .is_err());
    let valid = model(OBJECT, r#"<item objectid="1"/>"#);
    for target in [
        "https://example.invalid/model",
        "../outside.model",
        "/3D/%2e%2e/main.model",
    ] {
        assert!(load(source(archive(&valid, target)), ModelFormat::ThreeMf).is_err());
    }
    let mut resources = OBJECT.to_string();
    for id in 2..=20 {
        resources.push_str(&format!(r#"<object id="{id}"><components><component objectid="{}"/><component objectid="{}"/></components></object>"#,id-1,id-1));
    }
    assert!(load(
        source(archive(
            &model(&resources, r#"<item objectid="20"/>"#),
            "/3D/main.model"
        )),
        ModelFormat::ThreeMf
    )
    .is_err());
    let hostile = valid.replace(
        "<model ",
        "<!DOCTYPE model [<!ENTITY x SYSTEM 'file:///private'>]><model ",
    );
    assert!(load(
        source(archive(&hostile, "/3D/main.model")),
        ModelFormat::ThreeMf
    )
    .is_err());
}
#[test]
fn removal_truncation_and_geometry_limits_are_typed() {
    let input = source(b"solid test\nendsolid test".to_vec());
    input.removed.store(true, Ordering::SeqCst);
    assert!(matches!(
        load(input, ModelFormat::Stl),
        Err(PackError::MediaRemoved)
    ));
    assert!(load(source(vec![0; 16 * 1024 * 1024 + 1]), ModelFormat::Obj).is_err());
}
