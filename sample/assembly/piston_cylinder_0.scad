// @main_component Hollow Cylinder

// Functional print-in-place piston-cylinder assembly.
// All dimensions are millimetres. The cylinder and piston are disjoint solids.

$fn = 96;

bore_diameter = 30;
wall_thickness = 4;
cylinder_outer_diameter = bore_diameter + 2 * wall_thickness;
cylinder_length = 60;
rear_end_thickness = 4;
front_end_thickness = 4;
chamber_length = cylinder_length - rear_end_thickness - front_end_thickness;

piston_head_diameter = 29.2;
piston_head_thickness = 8;
radial_bore_clearance = (bore_diameter - piston_head_diameter) / 2;

rod_diameter = 10;
rod_guide_diameter = 10.8;
rod_length = 50;

// The piston head bottom may move from z=4 to z=44: exactly 40 mm.
// @param min=4 max=44 step=1 label=Piston_position
piston_z = 24;

epsilon = 0.2;

module hollow_cylinder() {
    difference() {
        cylinder(d = cylinder_outer_diameter, h = cylinder_length);

        // Axial 30 mm bore, leaving 4 mm integral walls at both ends.
        translate([0, 0, rear_end_thickness])
            cylinder(d = bore_diameter, h = chamber_length);

        // Coaxial rod guide through the front wall, with 0.4 mm radial clearance.
        translate([0, 0, cylinder_length - front_end_thickness - epsilon])
            cylinder(d = rod_guide_diameter,
                     h = front_end_thickness + 2 * epsilon);
    }
}

module piston_and_rod() {
    union() {
        // Head has 0.4 mm running clearance per side inside the bore.
        translate([0, 0, piston_z])
            cylinder(d = piston_head_diameter, h = piston_head_thickness);

        // Slight overlap joins the rod only to its own piston head.
        translate([0, 0, piston_z + piston_head_thickness - epsilon])
            cylinder(d = rod_diameter, h = rod_length + epsilon);
    }
}

hollow_cylinder();
piston_and_rod();

