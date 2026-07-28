use crate::protocol::{CadDiagnostic, CadDiagnostics, CadMesh, CadParameter, CadParameterValue};
use regex::Regex;
use std::collections::HashMap;

pub const DEFAULT_SAMPLE_SOURCE: &str = r#"width = 32; // @param min=8 max=80 step=1 label=Width
depth = 24; // @param min=8 max=80 step=1 label=Depth
height = 12; // @param min=4 max=60 step=1 label=Height

cube([width, depth, height]);
translate([24, 0, height]) cylinder(h=height * 2, r=6);
translate([-24, 0, 12]) sphere(r=8);
"#;

pub fn extract_open_scad_parameters(source: &str) -> Vec<CadParameter> {
    source
        .lines()
        .filter_map(|line| {
            let (assignment, comment) = line.split_once("// @param")?;
            let (name, value) = assignment.split_once('=')?;
            let literal = value.trim().trim_end_matches(';').trim();
            let mut parameter = CadParameter {
                name: name.trim().to_string(),
                value: parse_parameter_value(literal),
                parameter_type: parameter_type(literal).to_string(),
                min: None,
                max: None,
                step: None,
                label: None,
            };
            let tokens = split_param_tokens(comment.trim());
            let mut label_parts = Vec::new();
            let mut reading_label = false;
            for token in tokens {
                if let Some((key, value)) = token.split_once('=') {
                    reading_label = key == "label";
                    match key {
                        "min" => parameter.min = value.parse().ok(),
                        "max" => parameter.max = value.parse().ok(),
                        "step" => parameter.step = value.parse().ok(),
                        "label" => label_parts.push(value.to_string()),
                        _ => {}
                    }
                } else if reading_label {
                    label_parts.push(token.to_string());
                }
            }
            if !label_parts.is_empty() {
                parameter.label = Some(label_parts.join(" "));
            }
            Some(parameter)
        })
        .collect()
}

pub fn render_open_scad_preview(
    source: &str,
    parameters: &[CadParameter],
) -> (Option<CadMesh>, CadDiagnostics) {
    let started_at = std::time::Instant::now();
    let parsed = parse_open_scad(source, parameters);
    let mesh = (!parsed.primitives.is_empty()).then(|| build_combined_mesh(&parsed.primitives));
    let has_error = parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == "error");
    (
        mesh,
        CadDiagnostics {
            ok: !has_error && !parsed.primitives.is_empty(),
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            items: parsed.diagnostics,
        },
    )
}

pub fn ok_diagnostics(elapsed_ms: u64) -> CadDiagnostics {
    CadDiagnostics {
        ok: true,
        elapsed_ms,
        items: Vec::new(),
    }
}

fn split_param_tokens(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}

fn parse_parameter_value(literal: &str) -> CadParameterValue {
    if literal == "true" {
        return CadParameterValue::Boolean(true);
    }
    if literal == "false" {
        return CadParameterValue::Boolean(false);
    }
    if let Ok(number) = literal.parse::<f64>() {
        return CadParameterValue::Number(number);
    }
    CadParameterValue::String(literal.trim_matches('"').to_string())
}

fn parameter_type(literal: &str) -> &'static str {
    if literal == "true" || literal == "false" {
        "boolean"
    } else if literal.parse::<f64>().is_ok() {
        "number"
    } else {
        "string"
    }
}

#[derive(Clone, Debug)]
struct Primitive {
    kind: PrimitiveKind,
    translate: [f64; 3],
    size: Option<[f64; 3]>,
    radius: Option<f64>,
    height: Option<f64>,
}

#[derive(Clone, Debug)]
enum PrimitiveKind {
    Cube,
    Sphere,
    Cylinder,
}

struct ParsedOpenScad {
    primitives: Vec<Primitive>,
    diagnostics: Vec<CadDiagnostic>,
}

fn parse_open_scad(source: &str, parameters: &[CadParameter]) -> ParsedOpenScad {
    let mut diagnostics = Vec::new();
    let env = build_environment(source, parameters);
    let source_without_comments = strip_line_comments(source);
    let primitive_pattern = Regex::new(
        r"(?is)(?:translate\s*\(\s*(\[[^\]]+\])\s*\)\s*)?(cube|sphere|cylinder)\s*\(([^;]+)\)\s*;",
    )
    .expect("primitive regex is valid");
    let mut primitives = Vec::new();

    for capture in primitive_pattern.captures_iter(&source_without_comments) {
        let translate = capture
            .get(1)
            .and_then(|source| parse_vector(source.as_str(), &env))
            .unwrap_or([0.0, 0.0, 0.0]);
        let kind = capture
            .get(2)
            .map(|value| value.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        let args = capture
            .get(3)
            .map(|value| value.as_str())
            .unwrap_or_default();
        if let Some(primitive) = parse_primitive(&kind, args, translate, &env) {
            primitives.push(primitive);
        } else {
            diagnostics.push(CadDiagnostic {
                severity: "error".to_string(),
                message: format!("Could not parse {kind} arguments: {}", args.trim()),
                line: None,
                column: None,
            });
        }
    }

    if primitives.is_empty() {
        diagnostics.push(CadDiagnostic {
            severity: "error".to_string(),
            message: "No supported OpenSCAD primitives found. Supported MVP forms: cube, sphere, cylinder, translate(...).".to_string(),
            line: None,
            column: None,
        });
    }

    if Regex::new(r"(?i)\b(union|difference|intersection|rotate|scale|minkowski|hull)\s*\(")
        .expect("operation regex is valid")
        .is_match(&source_without_comments)
    {
        diagnostics.push(CadDiagnostic {
            severity: "warning".to_string(),
            message: "Some OpenSCAD operations are not evaluated by the MVP parser yet; supported primitives are previewed independently.".to_string(),
            line: None,
            column: None,
        });
    }

    ParsedOpenScad {
        primitives,
        diagnostics,
    }
}

fn parse_primitive(
    kind: &str,
    args_source: &str,
    translate: [f64; 3],
    env: &HashMap<String, f64>,
) -> Option<Primitive> {
    match kind {
        "cube" => {
            if let Some(vector_source) = find_vector(args_source) {
                return parse_vector(vector_source, env).map(|size| Primitive {
                    kind: PrimitiveKind::Cube,
                    translate,
                    size: Some(size),
                    radius: None,
                    height: None,
                });
            }
            let args = split_top_level(args_source);
            let size = value_for_named_or_positional(&args, "size", 0)
                .and_then(|value| parse_number_expression(value, env));
            size.map(|size| Primitive {
                kind: PrimitiveKind::Cube,
                translate,
                size: Some([size, size, size]),
                radius: None,
                height: None,
            })
        }
        "sphere" => {
            let args = split_top_level(args_source);
            let radius = value_for_named_or_positional(&args, "r", 0)
                .and_then(|value| parse_number_expression(value, env));
            radius.map(|radius| Primitive {
                kind: PrimitiveKind::Sphere,
                translate,
                size: None,
                radius: Some(radius),
                height: None,
            })
        }
        "cylinder" => {
            let args = split_top_level(args_source);
            let height = value_for_named_or_positional(&args, "h", 0)
                .and_then(|value| parse_number_expression(value, env));
            let radius = value_for_named_or_positional(&args, "r", 1)
                .or_else(|| value_for_named_or_positional(&args, "r1", 1))
                .and_then(|value| parse_number_expression(value, env));
            match (height, radius) {
                (Some(height), Some(radius)) => Some(Primitive {
                    kind: PrimitiveKind::Cylinder,
                    translate,
                    size: None,
                    radius: Some(radius),
                    height: Some(height),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map(|(code, _)| code).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_environment(source: &str, parameters: &[CadParameter]) -> HashMap<String, f64> {
    let mut env = HashMap::new();
    let assignment_pattern = Regex::new(r#"(?m)^\s*([A-Za-z_]\w*)\s*=\s*([-+]?\d*\.?\d+)\s*;"#)
        .expect("assignment regex is valid");
    for capture in assignment_pattern.captures_iter(source) {
        if let (Some(name), Some(value)) = (capture.get(1), capture.get(2)) {
            if let Ok(value) = value.as_str().parse::<f64>() {
                env.insert(name.as_str().to_string(), value);
            }
        }
    }
    for parameter in parameters {
        if let CadParameterValue::Number(value) = parameter.value {
            env.insert(parameter.name.clone(), value);
        }
    }
    env
}

fn find_vector(source: &str) -> Option<&str> {
    let start = source.find('[')?;
    let end = source[start..].find(']')?;
    Some(&source[start..=start + end])
}

fn parse_vector(source: &str, env: &HashMap<String, f64>) -> Option<[f64; 3]> {
    let parts = source
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    Some([
        parse_number_expression(parts[0], env)?,
        parse_number_expression(parts[1], env)?,
        parse_number_expression(parts[2], env)?,
    ])
}

fn split_top_level(source: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    for (index, character) in source.char_indices() {
        match character {
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            ',' if bracket_depth == 0 && paren_depth == 0 => {
                parts.push(source[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(source[start..].trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn value_for_named_or_positional<'a>(
    args: &'a [&'a str],
    name: &str,
    position: usize,
) -> Option<&'a str> {
    args.iter()
        .find_map(|arg| {
            let (candidate_name, value) = arg.split_once('=')?;
            (candidate_name.trim() == name).then_some(value.trim())
        })
        .or_else(|| {
            args.iter()
                .filter(|arg| !arg.contains('='))
                .nth(position)
                .copied()
        })
}

fn parse_number_expression(expression: &str, env: &HashMap<String, f64>) -> Option<f64> {
    let mut parser = ExpressionParser {
        input: expression.as_bytes(),
        index: 0,
        env,
    };
    let value = parser.parse_expression()?;
    parser.skip_whitespace();
    (parser.index == parser.input.len() && value.is_finite()).then_some(value)
}

struct ExpressionParser<'a> {
    input: &'a [u8],
    index: usize,
    env: &'a HashMap<String, f64>,
}

impl ExpressionParser<'_> {
    fn parse_expression(&mut self) -> Option<f64> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'+') => {
                    self.index += 1;
                    value += self.parse_term()?;
                }
                Some(b'-') => {
                    self.index += 1;
                    value -= self.parse_term()?;
                }
                _ => return Some(value),
            }
        }
    }

    fn parse_term(&mut self) -> Option<f64> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'*') => {
                    self.index += 1;
                    value *= self.parse_factor()?;
                }
                Some(b'/') => {
                    self.index += 1;
                    value /= self.parse_factor()?;
                }
                _ => return Some(value),
            }
        }
    }

    fn parse_factor(&mut self) -> Option<f64> {
        self.skip_whitespace();
        match self.peek()? {
            b'+' => {
                self.index += 1;
                self.parse_factor()
            }
            b'-' => {
                self.index += 1;
                Some(-self.parse_factor()?)
            }
            b'(' => {
                self.index += 1;
                let value = self.parse_expression()?;
                self.skip_whitespace();
                (self.peek()? == b')').then(|| self.index += 1)?;
                Some(value)
            }
            character if character.is_ascii_digit() || character == b'.' => self.parse_number(),
            character if character.is_ascii_alphabetic() || character == b'_' => {
                self.parse_identifier()
            }
            _ => None,
        }
    }

    fn parse_number(&mut self) -> Option<f64> {
        let start = self.index;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_digit() || character == b'.')
        {
            self.index += 1;
        }
        std::str::from_utf8(&self.input[start..self.index])
            .ok()?
            .parse::<f64>()
            .ok()
    }

    fn parse_identifier(&mut self) -> Option<f64> {
        let start = self.index;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == b'_')
        {
            self.index += 1;
        }
        let name = std::str::from_utf8(&self.input[start..self.index]).ok()?;
        self.env.get(name).copied()
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_whitespace())
        {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.index).copied()
    }
}

fn build_combined_mesh(primitives: &[Primitive]) -> CadMesh {
    let mut combined = CadMesh {
        vertices: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
    };
    for primitive in primitives {
        let mesh = match primitive.kind {
            PrimitiveKind::Cube => build_cube_mesh(primitive),
            PrimitiveKind::Sphere => build_sphere_mesh(primitive),
            PrimitiveKind::Cylinder => build_cylinder_mesh(primitive),
        };
        append_mesh(&mut combined, &mesh);
    }
    combined
}

fn build_cube_mesh(primitive: &Primitive) -> CadMesh {
    let [width, depth, height] = primitive.size.unwrap_or([1.0, 1.0, 1.0]);
    cuboid_mesh(width, depth, height, primitive.translate)
}

fn cuboid_mesh(width: f64, depth: f64, height: f64, translate: [f64; 3]) -> CadMesh {
    let x = width / 2.0;
    let y = depth / 2.0;
    let z = height / 2.0;
    let [tx, ty, tz] = translate;
    let vertices = vec![
        tx - x,
        ty - y,
        tz - z,
        tx + x,
        ty - y,
        tz - z,
        tx + x,
        ty + y,
        tz - z,
        tx - x,
        ty + y,
        tz - z,
        tx - x,
        ty - y,
        tz + z,
        tx + x,
        ty - y,
        tz + z,
        tx + x,
        ty + y,
        tz + z,
        tx - x,
        ty + y,
        tz + z,
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 4, 5, 0, 5, 1, 1, 5, 6, 1, 6, 2, 2, 6, 7, 2, 7, 3,
        3, 7, 4, 3, 4, 0,
    ];
    let mut normals = vec![0.0; vertices.len()];
    for index in (0..normals.len()).step_by(3) {
        normals[index + 2] = 1.0;
    }
    CadMesh {
        vertices,
        normals,
        indices,
    }
}

fn build_sphere_mesh(primitive: &Primitive) -> CadMesh {
    let radius = primitive.radius.unwrap_or(1.0);
    let [tx, ty, tz] = primitive.translate;
    let segments = 24;
    let rings = 12;
    let mut mesh = CadMesh {
        vertices: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
    };

    for ring in 0..=rings {
        let phi = (std::f64::consts::PI * ring as f64) / rings as f64;
        for segment in 0..=segments {
            let theta = (std::f64::consts::PI * 2.0 * segment as f64) / segments as f64;
            let nx = phi.sin() * theta.cos();
            let ny = phi.sin() * theta.sin();
            let nz = phi.cos();
            mesh.vertices
                .extend([tx + nx * radius, ty + ny * radius, tz + nz * radius]);
            mesh.normals.extend([nx, ny, nz]);
        }
    }

    for ring in 0..rings {
        for segment in 0..segments {
            let first = ring * (segments + 1) + segment;
            let second = first + segments + 1;
            mesh.indices.extend([
                first as u32,
                second as u32,
                (first + 1) as u32,
                second as u32,
                (second + 1) as u32,
                (first + 1) as u32,
            ]);
        }
    }

    mesh
}

fn build_cylinder_mesh(primitive: &Primitive) -> CadMesh {
    let radius = primitive.radius.unwrap_or(1.0);
    let height = primitive.height.unwrap_or(1.0);
    let [tx, ty, tz] = primitive.translate;
    let segments = 32;
    let mut mesh = CadMesh {
        vertices: Vec::new(),
        normals: Vec::new(),
        indices: Vec::new(),
    };
    let top_center = push_vertex(&mut mesh, [tx, ty, tz + height / 2.0], [0.0, 0.0, 1.0]);
    let bottom_center = push_vertex(&mut mesh, [tx, ty, tz - height / 2.0], [0.0, 0.0, -1.0]);
    let mut top = Vec::new();
    let mut bottom = Vec::new();

    for index in 0..segments {
        let theta = (std::f64::consts::PI * 2.0 * index as f64) / segments as f64;
        let nx = theta.cos();
        let ny = theta.sin();
        top.push(push_vertex(
            &mut mesh,
            [tx + nx * radius, ty + ny * radius, tz + height / 2.0],
            [nx, ny, 0.0],
        ));
        bottom.push(push_vertex(
            &mut mesh,
            [tx + nx * radius, ty + ny * radius, tz - height / 2.0],
            [nx, ny, 0.0],
        ));
    }

    for index in 0..segments {
        let next = (index + 1) % segments;
        mesh.indices.extend([
            top[index],
            bottom[index],
            top[next],
            top[next],
            bottom[index],
            bottom[next],
            top_center,
            top[index],
            top[next],
            bottom_center,
            bottom[next],
            bottom[index],
        ]);
    }

    mesh
}

fn push_vertex(mesh: &mut CadMesh, vertex: [f64; 3], normal: [f64; 3]) -> u32 {
    let index = (mesh.vertices.len() / 3) as u32;
    mesh.vertices.extend(vertex);
    mesh.normals.extend(normal);
    index
}

fn append_mesh(target: &mut CadMesh, source: &CadMesh) {
    let offset = (target.vertices.len() / 3) as u32;
    target.vertices.extend(&source.vertices);
    target.normals.extend(&source.normals);
    target
        .indices
        .extend(source.indices.iter().map(|index| index + offset));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sphere_source_as_sphere_mesh() {
        let source = "radius = 6; // @param min=1 max=20 step=1 label=Radius\nsphere(r = radius);";
        let parameters = extract_open_scad_parameters(source);
        let (mesh, diagnostics) = render_open_scad_preview(source, &parameters);

        assert!(diagnostics.ok);
        let mesh = mesh.unwrap();
        assert!(mesh.vertices.len() / 3 > 100);
        assert!(mesh.indices.len() / 3 > 100);
    }

    #[test]
    fn renders_translated_cube_bounds_from_source() {
        let (mesh, diagnostics) =
            render_open_scad_preview("translate([10, 0, 0]) cube([2, 4, 6]);", &[]);
        assert!(diagnostics.ok);
        let mesh = mesh.unwrap();
        let xs = mesh.vertices.iter().step_by(3).copied().collect::<Vec<_>>();
        assert_eq!(xs.iter().copied().fold(f64::INFINITY, f64::min), 9.0);
        assert_eq!(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max), 11.0);
    }
}
