// @main_component universal_joint_assembly
// Functional three-part universal joint, assembled at 30 degrees.
// FFF target: 0.4 mm nozzle, slicer supports enabled.

$fn = 64;

// @param min=20 max=45 step=5 label=Shaft angle (deg)
shaft_angle = 30;
// @param min=5.0 max=7.0 step=0.2 label=Journal diameter (mm)
journal_d = 6.0;
// @param min=6.4 max=7.4 step=0.1 label=Yoke bore diameter (mm)
bore_d = 6.8;
// @param min=12.2 max=13.0 step=0.1 label=Shaft socket diameter (mm)
shaft_socket_d = 12.4;

arm_axis_offset = 12;
arm_thickness = 6;
arm_width = 8;
fork_reach = 21;
hub_outer_d = 20;
hub_length = 19;
collar_outer_d = 24;
journal_span = 29;

module cylinder_x(h, d, center=false) {
    rotate([0, 90, 0]) cylinder(h=h, d=d, center=center);
}

// Local yoke: shaft points toward -X, fork opens toward the joint center.
// The two bearing bores share the local Z axis.
module yoke_local() {
    difference() {
        union() {
            // Cylindrical shaft hub and reinforced fork collar.
            translate([-40, 0, 0]) cylinder_x(hub_length, hub_outer_d);
            translate([-24, 0, 0]) cylinder_x(8, collar_outer_d);

            // Upper and lower arms with round bearing eyes.
            for (zsign = [-1, 1]) {
                translate([-20, -arm_width/2, zsign*arm_axis_offset-arm_thickness/2])
                    cube([fork_reach, arm_width, arm_thickness]);
                translate([0, 0, zsign*arm_axis_offset])
                    cylinder(h=arm_thickness, d=12, center=true);
            }
        }

        // 12 mm nominal shaft connection with printable insertion clearance.
        translate([-41, 0, 0]) cylinder_x(27, shaft_socket_d);

        // One continuous cutter guarantees exact coaxial bearing bores.
        cylinder(h=38, d=bore_d, center=true);

        // Relief at the fork root leaves room for articulation at 30 degrees.
        translate([-15, 0, 0]) sphere(d=14);
    }
}

// Output yoke is mirrored so its shaft extends away from the joint, then
// oriented so local X becomes the 30 degree output axis and local Z becomes
// the in-plane transverse spider axis.
module output_yoke_oriented() {
    rotate([0, 0, shaft_angle])
        rotate([90, 0, 0])
            mirror([1, 0, 0])
                yoke_local();
}

module spider() {
    union() {
        // Rounded compact torque-transfer core.
        sphere(d=10);

        // First opposing journal pair: global Z, captured by input yoke.
        cylinder(h=14, d=8, center=true);
        cylinder(h=journal_span, d=journal_d, center=true);

        // Second opposing journal pair: perpendicular in XY plane and
        // captured by the output yoke. Rotation gives vector
        // (-sin(angle), cos(angle), 0), perpendicular to the output shaft.
        rotate([0, 0, shaft_angle])
            rotate([90, 0, 0]) {
                cylinder(h=14, d=8, center=true);
                cylinder(h=journal_span, d=journal_d, center=true);
            }
    }
}

// Three intentionally independent printable parts in their assembled state.
color([0.18, 0.45, 0.80]) yoke_local();
color([0.92, 0.56, 0.16]) output_yoke_oriented();
color([0.75, 0.76, 0.78]) spider();

