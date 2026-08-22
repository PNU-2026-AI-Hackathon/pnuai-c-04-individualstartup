// @main_component L-shaped mounting bracket

// Required dimensions in millimetres
// @param min=60 max=120 step=5 label=Base width
base_width = 80;
// @param min=35 max=80 step=5 label=Base depth
base_depth = 50;
// @param min=40 max=90 step=5 label=Vertical height
vertical_height = 60;
// @param min=4 max=10 step=1 label=Plate thickness
plate_thickness = 6;

base_hole_diameter = 8;
vertical_hole_diameter = 12;
wall_y = base_depth - plate_thickness;
vertical_hole_z = plate_thickness + vertical_height / 2;

rib_thickness = 6;
rib_run = 18;
rib_height = 30;
rib_left_x = 4;
rib_right_x = base_width - 4 - rib_thickness;

// Positive embedding removes face-only contacts at CSG junctions.
embed = 0.4;
$fn = 64;

// Closed 2D right-triangle profile, linearly extruded to a 6 mm rib.
// Local profile X maps to global Y, profile Y maps to global Z,
// and extrusion Z maps to global X after rotation.
module gusset_rib(x0) {
    translate([
        x0,
        wall_y - rib_run,
        plate_thickness - embed
    ])
        rotate([90, 0, 90])
            linear_extrude(height = rib_thickness, convexity = 4)
                polygon(points = [
                    [0, 0],
                    [rib_run + embed, 0],
                    [rib_run + embed, rib_height + embed]
                ]);
}

module bracket_body() {
    union() {
        // Exact 80 x 50 x 6 mm base envelope.
        cube([base_width, base_depth, plate_thickness]);

        // The wall embeds 0.4 mm into the base and ends at exact Z = 66 mm.
        translate([0, wall_y, plate_thickness - embed])
            cube([
                base_width,
                plate_thickness,
                vertical_height + embed
            ]);

        // Symmetric integral reinforcing ribs; each embeds into base and wall.
        gusset_rib(rib_left_x);
        gusset_rib(rib_right_x);
    }
}

// Force evaluation of one closed CSG result before STL tessellation.
render(convexity = 10)
difference() {
    bracket_body();

    // Four diameter-8 mounting holes through the horizontal plate.
    for (x = [15, base_width - 15])
        for (y = [14, 34])
            translate([x, y, -1])
                cylinder(d = base_hole_diameter, h = plate_thickness + 2);

    // One diameter-12 center hole through the vertical plate along Y.
    translate([base_width / 2, base_depth + 1, vertical_hole_z])
        rotate([90, 0, 0])
            cylinder(d = vertical_hole_diameter, h = plate_thickness + 2);
}

