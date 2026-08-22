// @main_component lead_screw
// Tr20x4 right-handed lead screw with a separate topology-safe split nut.
// Single start means pitch = lead = 4 mm and travel = 4 mm/revolution.

$fn = 72;

// @param min=60 max=120 step=4 label=Screw length (mm)
screw_length = 80;
// @param min=3 max=6 step=0.5 label=Thread pitch (mm)
pitch = 4;
// @param min=18 max=24 step=1 label=Nominal diameter (mm)
nominal_diameter = 20;
// @param min=16 max=32 step=4 label=Nut length (mm)
nut_length = 24;

starts = 1;
lead = pitch / starts;
major_radius = nominal_diameter / 2;
thread_depth = 1.50;
minor_radius = major_radius - thread_depth;
included_angle = 30;
half_flank_angle = included_angle / 2;

male_crest_width = 0.80;
male_root_width = male_crest_width
                  + 2 * thread_depth * tan(half_flank_angle);

slices_per_turn = 36;
female_axial_clearance = 0.80;
female_bore_radius = 9.45;       // 0.95 mm clear of the 8.50 mm screw core
female_groove_outer = 10.70;     // 0.70 mm clear beyond the 10 mm crest
radial_engagement = major_radius - female_bore_radius; // 0.55 mm
split_slot_width = 2.40;         // six 0.4 mm nozzle widths

nut_center_z = screw_length / 2;
nut_z0 = nut_center_z - nut_length / 2;
nut_across_flats = 34;
nut_outer_radius = nut_across_flats / sqrt(3);

function polar_xy(r, a) = [r * cos(a), r * sin(a)];
function width_angle(w) = 360 * w / lead;

module arc_helical_trapezoid(r_inner, r_outer,
                             root_width, crest_width,
                             height, turns) {
    ra = width_angle(root_width) / 2;
    ca = width_angle(crest_width) / 2;
    linear_extrude(height=height,
                   twist=360 * turns,
                   slices=round(turns * slices_per_turn),
                   convexity=30)
        polygon(points=[
            polar_xy(r_outer, -ca),
            polar_xy(r_outer, -ca / 2),
            polar_xy(r_outer, 0),
            polar_xy(r_outer, ca / 2),
            polar_xy(r_outer, ca),
            polar_xy(r_inner, ra),
            polar_xy(r_inner, ra / 2),
            polar_xy(r_inner, 0),
            polar_xy(r_inner, -ra / 2),
            polar_xy(r_inner, -ra)
        ]);
}

module lead_screw() {
    union() {
        cylinder(h=screw_length, r=minor_radius + 0.10);
        arc_helical_trapezoid(
            minor_radius - 0.10,
            major_radius,
            male_root_width,
            male_crest_width,
            screw_length,
            screw_length / lead
        );
    }
}

module female_thread_void() {
    cutter_z0 = nut_z0 - lead;  // phase-aligned: 24 mm / 4 mm = 6 turns
    cutter_height = nut_length + 2 * lead;
    translate([0, 0, cutter_z0])
        union() {
            cylinder(h=cutter_height, r=female_bore_radius);
            arc_helical_trapezoid(
                female_bore_radius - 0.25,
                female_groove_outer,
                male_root_width + female_axial_clearance,
                male_crest_width + female_axial_clearance,
                cutter_height,
                cutter_height / lead
            );
        }
}

module axial_split_slot() {
    // Opens the threaded bore through +X while leaving one continuous C-shaped nut.
    translate([0, -split_slot_width / 2, nut_z0 - 0.10])
        cube([nut_outer_radius + 1,
              split_slot_width,
              nut_length + 0.20]);
}

module traveling_split_nut() {
    difference() {
        translate([0, 0, nut_z0])
            cylinder(h=nut_length, r=nut_outer_radius, $fn=6);
        female_thread_void();
        axial_split_slot();
    }
}

// Two independent solids, coaxial at the initial six-pitch engagement position.
lead_screw();
traveling_split_nut();

