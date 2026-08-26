// @main_component bearing_housing_body
// Topology-repaired pillow-block mechanical housing for FFF printing

$fn = 72;

// @param min=40 max=40 step=1 label=Bore diameter (mm)
bore_diameter = 40;
// @param min=8 max=8 step=1 label=Mounting hole diameter (mm)
mounting_hole_diameter = 8;
// @param min=8 max=8 step=1 label=Base thickness (mm)
base_thickness = 8;

base_x = 100;
base_y = 80;
base_corner_radius = 6;
bore_center_z = 32;
main_outer_radius = 28;
collar_outer_radius = 29;

// A hull of four overlapping vertical solids produces one rounded base body.
module solid_mounting_base() {
    hull() {
        for (x = [-base_x / 2 + base_corner_radius,
                  base_x / 2 - base_corner_radius])
            for (y = [-base_y / 2 + base_corner_radius,
                      base_y / 2 - base_corner_radius])
                translate([x, y, 0])
                    cylinder(h = base_thickness,
                             r = base_corner_radius,
                             center = false);
    }
}

module horizontal_cylinder(length, radius, y_position = 0) {
    translate([0, y_position, bore_center_z])
        rotate([90, 0, 0])
            cylinder(h = length, r = radius, center = true);
}

module overlapped_mounting_bosses() {
    // Start 1 mm below the base top so every boss shares real volume.
    for (x = [-40, 40])
        for (y = [-30, 30])
            translate([x, y, base_thickness - 1])
                cylinder(h = 5, r = 9, center = false);
}

module bearing_shell_and_reinforcement() {
    union() {
        // The main shell overlaps the base by 4 mm at its lowest point.
        horizontal_cylinder(34, main_outer_radius);

        // Each face collar overlaps the main shell axially by 4.5 mm.
        horizontal_cylinder(6, collar_outer_radius, -15.5);
        horizontal_cylinder(6, collar_outer_radius, 15.5);

        // Pedestal overlaps both the base and the cylindrical bearing shell.
        translate([0, 0, 20.5])
            cube([50, 32, 28], center = true);

        // Thick shoulders spread radial load toward the base.
        for (x = [-24, 24])
            translate([x, 0, 15])
                cube([12, 30, 18], center = true);

        // Four face ribs overlap the base, collars, shoulders, and shell.
        for (x = [-24, 24])
            for (y = [-16, 16])
                translate([x, y, 18.5])
                    cube([8, 7, 22], center = true);

        // Low-profile service boss, embedded 2.5 mm into the shell crown.
        translate([0, 0, 57.5])
            cylinder(h = 3.5, r = 6, center = false);
    }
}

module connected_positive_body() {
    union() {
        solid_mounting_base();
        overlapped_mounting_bosses();
        bearing_shell_and_reinforcement();
    }
}

difference() {
    connected_positive_body();

    // Exact 40 mm bore, extended beyond both bearing faces.
    translate([0, 0, bore_center_z])
        rotate([90, 0, 0])
            cylinder(h = 44, d = bore_diameter, center = true);

    // Four exact 8 mm holes through the base and reinforcing bosses.
    for (x = [-40, 40])
        for (y = [-30, 30])
            translate([x, y, -1])
                cylinder(h = 15,
                         d = mounting_hole_diameter,
                         center = false);
}

