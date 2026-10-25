//! The triangle mesh of a scanned world's collider, read from a binary glTF (GLB).
//!
//! World Labs Marble ships a collider mesh next to its splats, in the same frame as the PLY. This
//! is the smallest GLB reader that serves it: positions (float vec3) and triangle indices (u8,
//! u16 or u32, or none), over every mesh in the file, from the embedded binary chunk. Anything it
//! does not understand (an external buffer, sparse accessors, node transforms, non-triangle
//! primitives) is refused with a typed [`MeshError`] rather than guessed at, and every length,
//! offset and index in the file is checked before it is used.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::math::Vec3;

/// The largest GLB accepted.
pub const MAX_GLB_BYTES: usize = 128 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRIANGLES: usize = 4_000_000;
pub const MAX_VERTICES: usize = 4_000_000;

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4e4f_534a;
const CHUNK_BIN: u32 = 0x004e_4942;
const COMPONENT_U8: u32 = 5121;
const COMPONENT_U16: u32 = 5123;
const COMPONENT_U32: u32 = 5125;
const COMPONENT_F32: u32 = 5126;
const MODE_TRIANGLES: u32 = 4;
const LARGEST_STRIDE: usize = 252;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshError {
    TooLarge,
    /// Not a GLB: the magic is wrong or the file is shorter than its header.
    NotGlb,
    /// Only glTF 2.0 is read.
    Version,
    /// The header's length is longer than the file, or a chunk runs past the end.
    Truncated,
    /// The chunk layout is not a JSON chunk first, then at most one binary chunk.
    ChunkLayout,
    JsonInvalid,
    NoBinaryChunk,
    /// A buffer that names a file or URL instead of the embedded chunk.
    ExternalBuffer,
    SparseAccessor,
    /// A node with a matrix or translation, rotation or scale: the mesh would need moving.
    NodeTransform,
    /// A primitive that is not a triangle list.
    PrimitiveMode,
    NoPositions,
    /// An accessor with the wrong component type, element type or stride for its role.
    AccessorFormat,
    /// An accessor, view or buffer that points outside the data.
    OutOfRange,
    /// A triangle index at or past the vertex count.
    IndexOutOfRange,
    /// Index count is not a multiple of three.
    TriangleCount,
    NonFinite,
    TooManyVertices,
    TooManyTriangles,
    NoTriangles,
}

/// A triangle soup: vertices and index triples, all finite and all in range.
#[derive(Clone, Debug, PartialEq)]
pub struct ColliderMesh {
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
}

impl ColliderMesh {
    /// Builds a mesh from parts, applying the same checks as a file.
    pub fn from_triangles(
        vertices: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
    ) -> Result<Self, MeshError> {
        if vertices.len() > MAX_VERTICES {
            return Err(MeshError::TooManyVertices);
        }
        if triangles.len() > MAX_TRIANGLES {
            return Err(MeshError::TooManyTriangles);
        }
        if triangles.is_empty() {
            return Err(MeshError::NoTriangles);
        }
        if vertices.iter().any(|vertex| !vertex.is_finite()) {
            return Err(MeshError::NonFinite);
        }
        let vertex_count = vertices.len();
        if triangles
            .iter()
            .flatten()
            .any(|index| *index as usize >= vertex_count)
        {
            return Err(MeshError::IndexOutOfRange);
        }
        Ok(Self {
            vertices,
            triangles,
        })
    }

    /// Reads every triangle-list primitive of a GLB, in the file's own frame.
    pub fn from_glb(bytes: &[u8]) -> Result<Self, MeshError> {
        let (json, bin) = split_chunks(bytes)?;
        let document: Document =
            serde_json::from_slice(json).map_err(|_| MeshError::JsonInvalid)?;
        if document.nodes.iter().any(Node::is_transformed) {
            return Err(MeshError::NodeTransform);
        }
        if document.buffers.iter().any(|buffer| buffer.uri.is_some()) {
            return Err(MeshError::ExternalBuffer);
        }
        let mut vertices: Vec<Vec3> = Vec::new();
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        for primitive in document.meshes.iter().flat_map(|mesh| &mesh.primitives) {
            if primitive.mode.unwrap_or(MODE_TRIANGLES) != MODE_TRIANGLES {
                return Err(MeshError::PrimitiveMode);
            }
            let position = *primitive
                .attributes
                .get("POSITION")
                .ok_or(MeshError::NoPositions)?;
            let positions = document.read(position, bin, &[COMPONENT_F32], "VEC3", 12)?;
            let vertex_base = vertices.len();
            if vertex_base + positions.count > MAX_VERTICES {
                return Err(MeshError::TooManyVertices);
            }
            for at in 0..positions.count {
                let element = positions.element(at)?;
                vertices.push(Vec3::new(
                    read_f32(element, 0)?,
                    read_f32(element, 4)?,
                    read_f32(element, 8)?,
                ));
            }
            let indices: Vec<u32> = match primitive.indices {
                Some(accessor) => {
                    let view = document.read(
                        accessor,
                        bin,
                        &[COMPONENT_U8, COMPONENT_U16, COMPONENT_U32],
                        "SCALAR",
                        0,
                    )?;
                    if view.count > MAX_TRIANGLES * 3 {
                        return Err(MeshError::TooManyTriangles);
                    }
                    (0..view.count)
                        .map(|at| view.index(at))
                        .collect::<Result<_, _>>()?
                }
                None => (0..positions.count as u32).collect(),
            };
            if !indices.len().is_multiple_of(3) {
                return Err(MeshError::TriangleCount);
            }
            if triangles.len() + indices.len() / 3 > MAX_TRIANGLES {
                return Err(MeshError::TooManyTriangles);
            }
            let vertex_base = u32::try_from(vertex_base).map_err(|_| MeshError::TooManyVertices)?;
            for triple in indices.as_chunks::<3>().0 {
                let mut corners = [0u32; 3];
                for (corner, index) in corners.iter_mut().zip(triple) {
                    if *index as usize >= positions.count {
                        return Err(MeshError::IndexOutOfRange);
                    }
                    *corner = vertex_base + index;
                }
                triangles.push(corners);
            }
        }
        Self::from_triangles(vertices, triangles)
    }

    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices
    }

    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// The three corners of triangle `at`.
    pub fn corners(&self, at: usize) -> Option<[Vec3; 3]> {
        let [a, b, c] = *self.triangles.get(at)?;
        Some([
            *self.vertices.get(a as usize)?,
            *self.vertices.get(b as usize)?,
            *self.vertices.get(c as usize)?,
        ])
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, MeshError> {
    let end = at.checked_add(4).ok_or(MeshError::Truncated)?;
    let field: [u8; 4] = bytes
        .get(at..end)
        .and_then(|part| part.try_into().ok())
        .ok_or(MeshError::Truncated)?;
    Ok(u32::from_le_bytes(field))
}

fn read_f32(bytes: &[u8], at: usize) -> Result<f32, MeshError> {
    Ok(f32::from_bits(
        read_u32(bytes, at).map_err(|_| MeshError::OutOfRange)?,
    ))
}

/// The JSON chunk and the binary chunk (empty when the file has none) of a GLB.
fn split_chunks(bytes: &[u8]) -> Result<(&[u8], &[u8]), MeshError> {
    if bytes.len() > MAX_GLB_BYTES {
        return Err(MeshError::TooLarge);
    }
    if bytes.get(..4) != Some(&GLB_MAGIC[..]) || bytes.len() < 12 {
        return Err(MeshError::NotGlb);
    }
    if read_u32(bytes, 4)? != 2 {
        return Err(MeshError::Version);
    }
    let total = read_u32(bytes, 8)? as usize;
    if total > bytes.len() || total < 12 {
        return Err(MeshError::Truncated);
    }
    let mut json: Option<&[u8]> = None;
    let mut bin: Option<&[u8]> = None;
    let mut at = 12usize;
    while at < total {
        let length = read_u32(bytes, at)? as usize;
        let kind = read_u32(bytes, at + 4)?;
        let start = at + 8;
        let end = start.checked_add(length).ok_or(MeshError::Truncated)?;
        if end > total {
            return Err(MeshError::Truncated);
        }
        let data = bytes.get(start..end).ok_or(MeshError::Truncated)?;
        match (kind, json.is_some(), bin.is_some()) {
            (CHUNK_JSON, false, false) => {
                if length > MAX_JSON_BYTES {
                    return Err(MeshError::TooLarge);
                }
                json = Some(data);
            }
            (CHUNK_BIN, true, false) => bin = Some(data),
            // Chunks of unknown type are skipped, as the format allows.
            (CHUNK_JSON | CHUNK_BIN, ..) => return Err(MeshError::ChunkLayout),
            _ => {}
        }
        // Chunks are padded to four bytes.
        at = end
            .checked_add((4 - end % 4) % 4)
            .ok_or(MeshError::Truncated)?;
    }
    let json = json.ok_or(MeshError::ChunkLayout)?;
    Ok((json, bin.unwrap_or(&[])))
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    accessors: Vec<Accessor>,
    #[serde(default, rename = "bufferViews")]
    buffer_views: Vec<BufferView>,
    #[serde(default)]
    buffers: Vec<Buffer>,
    #[serde(default)]
    meshes: Vec<Mesh>,
    #[serde(default)]
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Accessor {
    #[serde(rename = "bufferView")]
    buffer_view: Option<usize>,
    #[serde(default, rename = "byteOffset")]
    byte_offset: usize,
    #[serde(rename = "componentType")]
    component_type: u32,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    sparse: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct BufferView {
    buffer: usize,
    #[serde(default, rename = "byteOffset")]
    byte_offset: usize,
    #[serde(rename = "byteLength")]
    byte_length: usize,
    #[serde(rename = "byteStride")]
    byte_stride: Option<usize>,
}

#[derive(Deserialize)]
struct Buffer {
    uri: Option<String>,
}

#[derive(Deserialize)]
struct Mesh {
    #[serde(default)]
    primitives: Vec<Primitive>,
}

#[derive(Deserialize)]
struct Primitive {
    #[serde(default)]
    attributes: BTreeMap<String, usize>,
    indices: Option<usize>,
    mode: Option<u32>,
}

#[derive(Deserialize)]
struct Node {
    matrix: Option<serde_json::Value>,
    translation: Option<serde_json::Value>,
    rotation: Option<serde_json::Value>,
    scale: Option<serde_json::Value>,
}

impl Node {
    fn is_transformed(&self) -> bool {
        self.matrix.is_some()
            || self.translation.is_some()
            || self.rotation.is_some()
            || self.scale.is_some()
    }
}

/// A checked window onto an accessor's elements.
struct View<'a> {
    data: &'a [u8],
    count: usize,
    stride: usize,
    element_bytes: usize,
    component_type: u32,
}

impl<'a> View<'a> {
    /// The bytes of element `at`.
    fn element(&self, at: usize) -> Result<&'a [u8], MeshError> {
        let start = at.checked_mul(self.stride).ok_or(MeshError::OutOfRange)?;
        let end = start
            .checked_add(self.element_bytes)
            .ok_or(MeshError::OutOfRange)?;
        self.data.get(start..end).ok_or(MeshError::OutOfRange)
    }

    /// A scalar index, widened to u32.
    fn index(&self, at: usize) -> Result<u32, MeshError> {
        let element = self.element(at)?;
        match self.component_type {
            COMPONENT_U8 => element.first().map(|byte| u32::from(*byte)),
            COMPONENT_U16 => element
                .get(..2)
                .and_then(|part| <[u8; 2]>::try_from(part).ok())
                .map(|part| u32::from(u16::from_le_bytes(part))),
            _ => read_u32(element, 0).ok(),
        }
        .ok_or(MeshError::OutOfRange)
    }
}

fn component_bytes(component_type: u32) -> Option<usize> {
    match component_type {
        COMPONENT_U8 => Some(1),
        COMPONENT_U16 => Some(2),
        COMPONENT_U32 | COMPONENT_F32 => Some(4),
        _ => None,
    }
}

impl Document {
    /// The elements of accessor `index`, checked against the roles `components`/`kind`. A vec3
    /// of floats passes `element_bytes` 12; a scalar passes 0 and is sized by its component.
    fn read<'a>(
        &self,
        index: usize,
        bin: &'a [u8],
        components: &[u32],
        kind: &str,
        element_bytes: usize,
    ) -> Result<View<'a>, MeshError> {
        let accessor = self.accessors.get(index).ok_or(MeshError::OutOfRange)?;
        if accessor.sparse.is_some() {
            return Err(MeshError::SparseAccessor);
        }
        if !components.contains(&accessor.component_type) || accessor.kind != kind {
            return Err(MeshError::AccessorFormat);
        }
        let element_bytes = if element_bytes == 0 {
            component_bytes(accessor.component_type).ok_or(MeshError::AccessorFormat)?
        } else {
            element_bytes
        };
        let view = self
            .buffer_views
            .get(accessor.buffer_view.ok_or(MeshError::OutOfRange)?)
            .ok_or(MeshError::OutOfRange)?;
        // Only the embedded chunk is buffer 0.
        if view.buffer != 0 || self.buffers.len() != 1 {
            return Err(MeshError::OutOfRange);
        }
        let stride = view.byte_stride.unwrap_or(element_bytes);
        if stride < element_bytes || stride > LARGEST_STRIDE {
            return Err(MeshError::AccessorFormat);
        }
        let view_end = view
            .byte_offset
            .checked_add(view.byte_length)
            .ok_or(MeshError::OutOfRange)?;
        let start = view
            .byte_offset
            .checked_add(accessor.byte_offset)
            .ok_or(MeshError::OutOfRange)?;
        // The last element needs only its own bytes, not a whole stride.
        let used = match accessor.count {
            0 => 0,
            count => (count - 1)
                .checked_mul(stride)
                .and_then(|span| span.checked_add(element_bytes))
                .ok_or(MeshError::OutOfRange)?,
        };
        let end = start.checked_add(used).ok_or(MeshError::OutOfRange)?;
        if end > view_end {
            return Err(MeshError::OutOfRange);
        }
        Ok(View {
            data: bin.get(start..end).ok_or(MeshError::OutOfRange)?,
            count: accessor.count,
            stride,
            element_bytes,
            component_type: accessor.component_type,
        })
    }
}

/// Test-only GLB writer, shared with the world grid's tests.
#[cfg(test)]
pub(super) mod test_glb {
    use super::*;

    /// One indexed triangle-list mesh with u32 indices, no node transforms.
    pub fn encode(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        for triangle in triangles {
            for index in triangle {
                bin.extend_from_slice(&index.to_le_bytes());
            }
        }
        let index_bytes = bin.len();
        for vertex in vertices {
            for value in [vertex.x, vertex.y, vertex.z] {
                bin.extend_from_slice(&value.to_le_bytes());
            }
        }
        let position_bytes = bin.len() - index_bytes;
        let json = format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
             \"nodes\":[{{\"mesh\":0}}],\
             \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":1}},\"indices\":0,\"mode\":4}}]}}],\
             \"accessors\":[{{\"bufferView\":0,\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}},\
             {{\"bufferView\":1,\"componentType\":5126,\"count\":{},\"type\":\"VEC3\"}}],\
             \"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{index_bytes}}},\
             {{\"buffer\":0,\"byteOffset\":{index_bytes},\"byteLength\":{position_bytes}}}],\
             \"buffers\":[{{\"byteLength\":{}}}]}}",
            triangles.len() * 3,
            vertices.len(),
            bin.len()
        );
        wrap(json.as_bytes(), &bin)
    }

    /// A GLB around arbitrary JSON and binary chunks, padded as the format requires.
    pub fn wrap(json: &[u8], bin: &[u8]) -> Vec<u8> {
        let pad = |mut data: Vec<u8>, with: u8| {
            while !data.len().is_multiple_of(4) {
                data.push(with);
            }
            data
        };
        let json = pad(json.to_vec(), b' ');
        let bin = pad(bin.to_vec(), 0);
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_glb::{encode, wrap};
    use super::*;

    fn square() -> (Vec<Vec3>, Vec<[u32; 3]>) {
        (
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, 2.0),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    #[test]
    fn a_synthetic_glb_round_trips() {
        let (vertices, triangles) = square();
        let mesh = ColliderMesh::from_glb(&encode(&vertices, &triangles)).unwrap();
        assert_eq!(mesh.vertices(), vertices);
        assert_eq!(mesh.triangles(), triangles);
        assert_eq!(
            mesh.corners(1),
            Some([vertices[0], vertices[2], vertices[3]])
        );
        assert_eq!(mesh.corners(2), None);
    }

    #[test]
    fn u16_indices_a_stride_and_a_second_primitive_are_read() {
        // Positions interleaved with 4 bytes of padding per vertex (stride 16), u16 indices.
        let mut bin: Vec<u8> = Vec::new();
        for (x, z) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0)] {
            for value in [x, 0.0, z] {
                bin.extend_from_slice(&value.to_le_bytes());
            }
            bin.extend_from_slice(&[0xAA; 4]);
        }
        let indices_at = bin.len();
        for index in [0u16, 1, 2] {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        bin.extend_from_slice(&[0, 0]);
        let json = format!(
            "{{\"meshes\":[{{\"primitives\":[\
             {{\"attributes\":{{\"POSITION\":0}},\"indices\":1}},\
             {{\"attributes\":{{\"POSITION\":0}}}}]}}],\
             \"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},\
             {{\"bufferView\":1,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}],\
             \"bufferViews\":[{{\"buffer\":0,\"byteLength\":{indices_at},\"byteStride\":16}},\
             {{\"buffer\":0,\"byteOffset\":{indices_at},\"byteLength\":6}}],\
             \"buffers\":[{{\"byteLength\":{}}}]}}",
            bin.len()
        );
        let mesh = ColliderMesh::from_glb(&wrap(json.as_bytes(), &bin)).unwrap();
        // The first primitive is indexed; the second has no indices, so its three vertices are
        // one triangle. The vertices of both are kept: 6 in all.
        assert_eq!(mesh.vertices().len(), 6);
        assert_eq!(mesh.triangles(), [[0, 1, 2], [3, 4, 5]]);
        assert_eq!(mesh.vertices()[1], Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn from_triangles_checks_what_a_file_would() {
        let (vertices, triangles) = square();
        assert!(ColliderMesh::from_triangles(vertices.clone(), triangles.clone()).is_ok());
        assert_eq!(
            ColliderMesh::from_triangles(vertices.clone(), vec![[0, 1, 4]]),
            Err(MeshError::IndexOutOfRange)
        );
        assert_eq!(
            ColliderMesh::from_triangles(vertices.clone(), vec![]),
            Err(MeshError::NoTriangles)
        );
        let mut bad = vertices;
        bad[2] = Vec3::new(f32::NAN, 0.0, 0.0);
        assert_eq!(
            ColliderMesh::from_triangles(bad, triangles),
            Err(MeshError::NonFinite)
        );
    }

    #[test]
    fn a_file_that_is_not_a_glb_is_refused() {
        assert_eq!(ColliderMesh::from_glb(&[]), Err(MeshError::NotGlb));
        assert_eq!(ColliderMesh::from_glb(b"glTF"), Err(MeshError::NotGlb));
        assert_eq!(
            ColliderMesh::from_glb(b"ply\nformat binary_little_endian 1.0\n"),
            Err(MeshError::NotGlb)
        );
        let (vertices, triangles) = square();
        let mut version_one = encode(&vertices, &triangles);
        version_one[4] = 1;
        assert_eq!(
            ColliderMesh::from_glb(&version_one),
            Err(MeshError::Version)
        );
    }

    #[test]
    fn every_truncation_of_a_valid_glb_is_refused_without_panicking() {
        let (vertices, triangles) = square();
        let bytes = encode(&vertices, &triangles);
        for cut in 0..bytes.len() {
            assert!(
                ColliderMesh::from_glb(&bytes[..cut]).is_err(),
                "cut at {cut} was accepted"
            );
        }
        assert!(ColliderMesh::from_glb(&bytes).is_ok());
    }

    #[test]
    fn a_header_length_or_chunk_length_past_the_data_is_refused() {
        let (vertices, triangles) = square();
        let good = encode(&vertices, &triangles);
        let mut long_total = good.clone();
        long_total[8..12].copy_from_slice(&(good.len() as u32 + 8).to_le_bytes());
        assert_eq!(
            ColliderMesh::from_glb(&long_total),
            Err(MeshError::Truncated)
        );
        let mut long_chunk = good.clone();
        long_chunk[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            ColliderMesh::from_glb(&long_chunk),
            Err(MeshError::Truncated)
        );
        let mut json_too_big = good;
        json_too_big[12..16].copy_from_slice(&(MAX_JSON_BYTES as u32 + 4).to_le_bytes());
        assert!(ColliderMesh::from_glb(&json_too_big).is_err());
    }

    #[test]
    fn a_missing_json_chunk_or_binary_chunk_is_refused() {
        // A binary chunk first.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&20u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        assert_eq!(ColliderMesh::from_glb(&bytes), Err(MeshError::ChunkLayout));
        // Nothing after the header.
        let mut header_only = Vec::new();
        header_only.extend_from_slice(b"glTF");
        header_only.extend_from_slice(&2u32.to_le_bytes());
        header_only.extend_from_slice(&12u32.to_le_bytes());
        assert_eq!(
            ColliderMesh::from_glb(&header_only),
            Err(MeshError::ChunkLayout)
        );
        // Valid JSON naming an accessor but no binary chunk to hold it.
        let json = br#"{"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
            "bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]}"#;
        let mut no_bin = wrap(json, &[]);
        // Drop the empty binary chunk header entirely.
        no_bin.truncate(no_bin.len() - 8);
        let total = no_bin.len() as u32;
        no_bin[8..12].copy_from_slice(&total.to_le_bytes());
        assert_eq!(ColliderMesh::from_glb(&no_bin), Err(MeshError::OutOfRange));
    }

    #[test]
    fn bad_json_is_refused() {
        assert_eq!(
            ColliderMesh::from_glb(&wrap(b"{not json", &[0; 4])),
            Err(MeshError::JsonInvalid)
        );
        assert_eq!(
            ColliderMesh::from_glb(&wrap(b"[1,2,3]", &[0; 4])),
            Err(MeshError::JsonInvalid)
        );
        // Valid but empty: no meshes means no triangles.
        assert_eq!(
            ColliderMesh::from_glb(&wrap(b"{}", &[0; 4])),
            Err(MeshError::NoTriangles)
        );
    }

    /// The valid mesh JSON with one piece swapped, around a binary chunk big enough for it.
    fn glb_with(swap: impl Fn(String) -> String) -> Vec<u8> {
        let (vertices, triangles) = square();
        let good = encode(&vertices, &triangles);
        let json_length = u32::from_le_bytes(good[12..16].try_into().unwrap()) as usize;
        let json = String::from_utf8(good[20..20 + json_length].to_vec()).unwrap();
        let bin = good[20 + json_length + 8..].to_vec();
        wrap(swap(json.trim_end().to_owned()).as_bytes(), &bin)
    }

    #[test]
    fn unsupported_structures_are_refused_by_name() {
        let cases: [(&str, &str, MeshError); 7] = [
            ("\"mode\":4", "\"mode\":1", MeshError::PrimitiveMode),
            (
                "\"buffers\":[{\"byteLength\":",
                "\"buffers\":[{\"uri\":\"x.bin\",\"byteLength\":",
                MeshError::ExternalBuffer,
            ),
            (
                "\"nodes\":[{\"mesh\":0}]",
                "\"nodes\":[{\"mesh\":0,\"scale\":[2,2,2]}]",
                MeshError::NodeTransform,
            ),
            (
                "\"nodes\":[{\"mesh\":0}]",
                "\"nodes\":[{\"mesh\":0,\"matrix\":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}]",
                MeshError::NodeTransform,
            ),
            (
                "\"componentType\":5125",
                "\"componentType\":5126",
                MeshError::AccessorFormat,
            ),
            (
                "\"type\":\"VEC3\"",
                "\"type\":\"VEC4\"",
                MeshError::AccessorFormat,
            ),
            (
                "\"count\":4,\"type\":\"VEC3\"",
                "\"count\":4,\"type\":\"VEC3\",\"sparse\":{}",
                MeshError::SparseAccessor,
            ),
        ];
        for (from, to, expected) in cases {
            let bytes = glb_with(|json| {
                assert!(json.contains(from), "fixture lost {from}");
                json.replace(from, to)
            });
            assert_eq!(ColliderMesh::from_glb(&bytes), Err(expected), "{to}");
        }
    }

    #[test]
    fn accessors_that_point_outside_the_data_are_refused() {
        let cases: [(&str, &str); 4] = [
            // More positions than the view holds.
            (
                "\"count\":4,\"type\":\"VEC3\"",
                "\"count\":5,\"type\":\"VEC3\"",
            ),
            // More indices than the view holds (a multiple of three, so not a count error).
            (
                "\"count\":6,\"type\":\"SCALAR\"",
                "\"count\":9,\"type\":\"SCALAR\"",
            ),
            // A view that starts far beyond the chunk.
            ("\"byteOffset\":24", "\"byteOffset\":4000000000"),
            // An accessor index that does not exist.
            ("\"POSITION\":1", "\"POSITION\":9"),
        ];
        for (from, to) in cases {
            let bytes = glb_with(|json| {
                assert!(json.contains(from), "fixture lost {from}");
                json.replace(from, to)
            });
            assert_eq!(
                ColliderMesh::from_glb(&bytes),
                Err(MeshError::OutOfRange),
                "{to}"
            );
        }
    }

    #[test]
    fn an_index_past_the_vertices_or_a_ragged_triangle_list_is_refused() {
        let (vertices, _) = square();
        assert_eq!(
            ColliderMesh::from_glb(&encode(&vertices, &[[0, 1, 4]])),
            Err(MeshError::IndexOutOfRange)
        );
        let ragged = glb_with(|json| {
            json.replace(
                "\"count\":6,\"type\":\"SCALAR\"",
                "\"count\":5,\"type\":\"SCALAR\"",
            )
        });
        assert_eq!(
            ColliderMesh::from_glb(&ragged),
            Err(MeshError::TriangleCount)
        );
    }

    #[test]
    fn a_non_finite_position_is_refused() {
        let (mut vertices, triangles) = square();
        vertices[1].y = f32::INFINITY;
        assert_eq!(
            ColliderMesh::from_glb(&encode(&vertices, &triangles)),
            Err(MeshError::NonFinite)
        );
    }

    #[test]
    fn a_stride_smaller_than_an_element_is_refused() {
        let bytes = glb_with(|json| {
            json.replace(
                "{\"buffer\":0,\"byteOffset\":24,",
                "{\"buffer\":0,\"byteStride\":8,\"byteOffset\":24,",
            )
        });
        assert_eq!(
            ColliderMesh::from_glb(&bytes),
            Err(MeshError::AccessorFormat)
        );
    }

    #[test]
    fn an_oversize_glb_is_refused_by_length_alone() {
        let mut bytes = vec![0u8; MAX_GLB_BYTES + 1];
        bytes[..4].copy_from_slice(b"glTF");
        assert_eq!(ColliderMesh::from_glb(&bytes), Err(MeshError::TooLarge));
    }
}
