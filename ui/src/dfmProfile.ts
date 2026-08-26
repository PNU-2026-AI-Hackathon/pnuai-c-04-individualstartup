export const DFM_PROFILE_CATEGORIES = ["Printer", "Filament", "Quality", "Support", "Speed", "Other"] as const;

export type DfmProfileCategory = (typeof DFM_PROFILE_CATEGORIES)[number];
export type DfmProfileValueType = "number" | "boolean" | "enum" | "percent" | "multi" | "text";

export interface DfmProfileEntry {
  key: string;
  value: string;
  category: DfmProfileCategory;
  valueType: DfmProfileValueType;
  options?: string[];
}

export interface DfmProfileSyntaxResult {
  entries: DfmProfileEntry[];
  errors: string[];
}

export const REQUIRED_DFM_PROFILE_KEYS = [
  "printer_technology",
  "nozzle_diameter",
  "filament_diameter",
  "layer_height",
  "gcode_flavor"
] as const;

const BOOLEAN_KEYS = new Set([
  "automatic_extrusion_widths",
  "avoid_crossing_curled_overhangs",
  "avoid_crossing_perimeters",
  "binary_gcode",
  "complete_objects",
  "cooling",
  "dont_support_bridges",
  "enable_dynamic_fan_speeds",
  "enable_dynamic_overhang_speeds",
  "external_perimeters_first",
  "extra_perimeters_on_overhangs",
  "fan_always_on",
  "gap_fill_enabled",
  "gcode_comments",
  "high_current_on_filament_swap",
  "infill_first",
  "interface_shells",
  "ironing",
  "nozzle_high_flow",
  "only_retract_when_crossing_perimeters",
  "ooze_prevention",
  "overhangs",
  "prefer_clockwise_movements",
  "remaining_times",
  "retract_layer_change",
  "support_material",
  "support_material_auto",
  "support_material_buildplate_only",
  "support_material_with_sheath",
  "thick_bridges",
  "thin_walls",
  "wipe",
  "wipe_into_infill",
  "wipe_into_objects"
]);

const ENUM_OPTIONS: Record<string, string[]> = {
  arc_fitting: ["disabled", "emit_center", "emit_radius"],
  bottom_fill_pattern: ["monotonic", "monotonicline", "rectilinear", "concentric", "hilbertcurve", "archimedeanchords", "octagramspiral"],
  brim_type: ["no_brim", "outer_only", "inner_only", "outer_and_inner"],
  draft_shield: ["disabled", "limited", "enabled"],
  ensure_vertical_shell_thickness: ["disabled", "partial", "enabled"],
  fill_pattern: ["rectilinear", "alignedrectilinear", "grid", "triangles", "stars", "cubic", "adaptivecubic", "supportcubic", "honeycomb", "gyroid", "hilbertcurve", "archimedeanchords", "octagramspiral", "lightning", "concentric"],
  fuzzy_skin: ["none", "external", "all", "allwalls"],
  gcode_flavor: ["reprap", "reprapfirmware", "marlin", "marlin2", "klipper", "smoothie", "mach3", "machinekit", "no-extrusion"],
  host_type: ["octoprint", "duet", "flashair", "astrobox", "repetier", "mks", "prusalink", "prusaconnect"],
  ironing_type: ["top", "topmost", "solid", "all"],
  machine_limits_usage: ["emit_to_gcode", "time_estimate_only", "limits", "ignore"],
  perimeter_generator: ["classic", "arachne"],
  printer_technology: ["FFF", "SLA"],
  support_material_pattern: ["rectilinear", "rectilinear-grid", "honeycomb"],
  support_material_style: ["grid", "snug", "organic"]
};

export function parseDfmProfile(contents: string): DfmProfileSyntaxResult {
  const entries: DfmProfileEntry[] = [];
  const errors: string[] = [];
  const seen = new Set<string>();
  const lines = contents.replace(/\r\n?/g, "\n").split("\n");

  lines.forEach((line, index) => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith(";")) return;
    const separator = line.indexOf("=");
    if (separator < 1) {
      errors.push(`Line ${index + 1}: expected key = value.`);
      return;
    }
    const key = line.slice(0, separator).trim();
    const value = line.slice(separator + 1).trim();
    if (!/^[A-Za-z0-9_.-]+$/.test(key)) {
      errors.push(`Line ${index + 1}: invalid key “${key}”.`);
      return;
    }
    if (seen.has(key)) {
      errors.push(`Line ${index + 1}: duplicate key “${key}”.`);
      return;
    }
    seen.add(key);
    if (key === "bed_shape") {
      try {
        formatBedShapeDimensions(value);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        errors.push(`Line ${index + 1}: ${message}`);
        return;
      }
    }
    const options = ENUM_OPTIONS[key];
    entries.push({
      key,
      value,
      category: categoryForProfileKey(key),
      valueType: profileValueType(key, value),
      ...(options ? { options } : {})
    });
  });

  for (const key of REQUIRED_DFM_PROFILE_KEYS) {
    const entry = entries.find((candidate) => candidate.key === key);
    if (!entry) errors.push(`Required setting “${key}” is missing.`);
    else if (!entry.value) errors.push(`Required setting “${key}” cannot be empty.`);
  }

  return { entries, errors };
}

export function formatBedShapeDimensions(value: string): string {
  const numberPattern = "[+-]?(?:\\d+(?:\\.\\d*)?|\\.\\d+)(?:[eE][+-]?\\d+)?";
  const coordinatePattern = new RegExp(`^(${numberPattern})x(${numberPattern})$`);
  const points = value.split(",").map((point) => {
    const match = coordinatePattern.exec(point.trim());
    if (!match) throw new Error(`Invalid bed_shape coordinate “${point.trim()}”.`);
    const x = Number(match[1]);
    const y = Number(match[2]);
    if (!Number.isFinite(x) || !Number.isFinite(y)) {
      throw new Error("bed_shape coordinates must be finite numbers.");
    }
    return { x, y };
  });
  if (points.length < 3) throw new Error("bed_shape must contain at least three coordinates.");

  const xCoordinates = points.map(({ x }) => x);
  const yCoordinates = points.map(({ y }) => y);
  const width = Math.max(...xCoordinates) - Math.min(...xCoordinates);
  const height = Math.max(...yCoordinates) - Math.min(...yCoordinates);
  return `${width}x${height}`;
}

export function updateDfmProfileValue(contents: string, key: string, value: string): string {
  const lines = contents.replace(/\r\n?/g, "\n").split("\n");
  let replacements = 0;
  const nextLines = lines.map((line) => {
    const separator = line.indexOf("=");
    if (separator < 1 || line.slice(0, separator).trim() !== key) return line;
    replacements += 1;
    return `${line.slice(0, separator + 1)} ${value}`;
  });
  if (replacements !== 1) {
    throw new Error(`Expected exactly one profile setting named “${key}”; found ${replacements}.`);
  }
  return nextLines.join("\n");
}

export function macAppExecutablePath(path: string): string | null {
  const normalized = path.trim().replace(/\/$/, "");
  return normalized.toLowerCase().endsWith(".app")
    ? `${normalized}/Contents/MacOS/PrusaSlicer`
    : null;
}

function categoryForProfileKey(key: string): DfmProfileCategory {
  if (/^(printer_|bed_|machine_|gcode_|nozzle_|extruder_|retract_|deretract_|start_gcode|end_gcode|before_layer_gcode|layer_gcode|toolchange_gcode|pause_print_gcode|color_change_gcode|max_print_height)/.test(key)) return "Printer";
  if (/^(filament_|temperature|first_layer_temperature|bed_temperature|first_layer_bed_temperature|cooling|fan_|min_fan|max_fan|full_fan|disable_fan|chamber_)/.test(key)) return "Filament";
  if (/^(support_|raft_|brim_|skirt_|dont_support|interface_)/.test(key)) return "Support";
  if (/(^|_)speed($|_)|acceleration|feedrate|jerk|volumetric/.test(key)) return "Speed";
  if (/layer_height|perimeter|infill|fill_|extrusion_width|resolution|ironing|fuzzy|overhang|bridge|seam|thin_walls|elefant|ensure_vertical|arc_fitting/.test(key)) return "Quality";
  return "Other";
}

function profileValueType(key: string, value: string): DfmProfileValueType {
  if (ENUM_OPTIONS[key]) return "enum";
  if (BOOLEAN_KEYS.has(key)) return "boolean";
  if (/^-?(?:\d+(?:\.\d+)?|\.\d+)%$/.test(value)) return "percent";
  if (value.includes(",") && !/^".*"$/.test(value)) return "multi";
  if (/^-?(?:\d+(?:\.\d+)?|\.\d+)$/.test(value)) return "number";
  return "text";
}
