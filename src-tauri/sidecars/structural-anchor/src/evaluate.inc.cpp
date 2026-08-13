std::array<double, 3> normalize_aspect(const std::array<double, 3>& values) {
    const double max_value = std::max({values[0], values[1], values[2]});
    if (max_value <= 0.0) {
        return {0.0, 0.0, 0.0};
    }
    return {values[0] / max_value, values[1] / max_value, values[2] / max_value};
}

Json number_array(const std::array<double, 3>& values) {
    return Json::array({Json::number(values[0]), Json::number(values[1]), Json::number(values[2])});
}

Json check_json(
    const std::string& name,
    bool passed,
    const std::string& severity,
    const std::string& message,
    std::map<std::string, Json> details = {}
) {
    std::map<std::string, Json> object{
        {"message", Json::string(message)},
        {"name", Json::string(name)},
        {"passed", Json::boolean(passed)},
        {"severity", Json::string(severity)},
    };
    for (auto& [key, value] : details) {
        object.emplace(key, std::move(value));
    }
    return Json::object(std::move(object));
}

std::string reason_for_failed_check(const std::vector<Json>& checks) {
    for (const Json& check : checks) {
        if (!get_bool(check, "passed", false)) {
            return get_string(check, "name") + "_failed";
        }
    }
    return "";
}

bool has_error_diagnostic(const Json& diagnostics) {
    if (get_bool(diagnostics, "ok", false) == false) {
        return true;
    }
    const Json* items = diagnostics.get("items");
    if (!items || !items->is_array()) {
        return false;
    }
    for (const Json& item : items->array_value) {
        if (get_string(item, "severity") == "error") {
            return true;
        }
    }
    return false;
}

std::string metadata_string(const Json& manifest, const std::string& key) {
    if (const Json* value = manifest.get(key); value && value->is_string()) {
        return value->string_value;
    }
    if (const Json* metadata = manifest.get("metadata"); metadata && metadata->is_object()) {
        if (const Json* value = metadata->get(key); value && value->is_string()) {
            return value->string_value;
        }
    }
    return "";
}

double metadata_number(const Json& manifest, const std::string& key, double default_value = 0.0) {
    if (const Json* value = manifest.get(key); value && value->is_number()) {
        return value->number_value;
    }
    if (const Json* metadata = manifest.get("metadata"); metadata && metadata->is_object()) {
        if (const Json* value = metadata->get(key); value && value->is_number()) {
            return value->number_value;
        }
    }
    return default_value;
}

std::string source_text_from_input(const Json& input, const Json& manifest, const fs::path& base_dir) {
    std::string source = get_string(input, "sourceText");
    if (!source.empty()) {
        return source;
    }
    const std::string source_path = get_string(input, "sourcePath");
    if (!source_path.empty()) {
        return read_text_file(resolve_path(base_dir, source_path));
    }
    source = metadata_string(manifest, "sourceText");
    if (!source.empty()) {
        return source;
    }
    const std::string annotation = metadata_string(manifest, "mainComponentAnnotation");
    if (!annotation.empty()) {
        return annotation;
    }
    const std::string main_component = metadata_string(manifest, "mainComponent");
    if (!main_component.empty()) {
        return "// @main_component " + main_component;
    }
    return "";
}

Json evaluate(const Json& input, const fs::path& base_dir) {
    Json plan = load_json_field_or_path(input, base_dir, "plan", "planPath");
    Json manifest = load_json_field_or_path(input, base_dir, "artifactManifest", "artifactManifestPath");
    Json diagnostics = load_json_field_or_path(input, base_dir, "runtimeDiagnostics", "runtimeDiagnosticsPath");

    const std::string run_id = get_string(input, "runId");
    const std::string revision_id = get_string(input, "revisionId");
    std::string artifact_id = get_string(input, "artifactId");
    if (artifact_id.empty()) {
        artifact_id = get_string(manifest, "id");
    }
    const std::string stl_path_text = get_string(input, "stlPath");
    if (stl_path_text.empty()) {
        throw std::runtime_error("input.stlPath is required");
    }
    const fs::path stl_path = resolve_path(base_dir, stl_path_text);

    const Json* main_component = plan.get("mainComponent");
    if (!main_component || !main_component->is_object()) {
        throw std::runtime_error("plan.mainComponent is required");
    }
    const std::string main_name = get_string(*main_component, "name");
    if (main_name.empty()) {
        throw std::runtime_error("plan.mainComponent.name is required");
    }
    const Json* expected_aspect = plan.get("expectedAspectRatio");
    if (!expected_aspect || !expected_aspect->is_object()) {
        throw std::runtime_error("plan.expectedAspectRatio is required");
    }
    const std::array<double, 3> expected = {
        get_number(*expected_aspect, "x"),
        get_number(*expected_aspect, "y"),
        get_number(*expected_aspect, "z"),
    };
    const double tolerance = get_number(*expected_aspect, "tolerance", 0.3);
    const Json* runtime_constraints = plan.get("runtimeConstraints");
    std::string required_annotation = "// @main_component " + main_name;
    if (runtime_constraints && runtime_constraints->is_object()) {
        required_annotation = get_string(*runtime_constraints, "mainComponentAnnotation", required_annotation);
    }

    const std::string source_text = source_text_from_input(input, manifest, base_dir);
    const bool main_component_passed = source_text.find(required_annotation) != std::string::npos;

    std::vector<Json> checks;
    checks.push_back(check_json(
        "main_component",
        main_component_passed,
        main_component_passed ? "info" : "error",
        main_component_passed
            ? "Final source declares the Plan main component convention."
            : "Final source or artifact metadata does not declare the Plan main component convention.",
        {
            {"expectedAnnotation", Json::string(required_annotation)},
            {"mainComponent", Json::string(main_name)},
        }
    ));

    const bool runtime_passed = !has_error_diagnostic(diagnostics);
    checks.push_back(check_json(
        "runtime_diagnostics",
        runtime_passed,
        runtime_passed ? "info" : "error",
        runtime_passed ? "Preview and export diagnostics are clean." : "Runtime diagnostics contain errors.",
        {
            {"ok", Json::boolean(get_bool(diagnostics, "ok", false))},
            {"elapsedMs", Json::number(get_number(diagnostics, "elapsedMs", get_number(diagnostics, "elapsed_ms", 0.0)))},
        }
    ));

    bool file_exists = fs::exists(stl_path);
    MeshStats mesh;
    if (file_exists) {
        mesh = load_mesh(stl_path);
    } else {
        mesh.error = "STL file does not exist.";
    }

    const double expected_bytes = metadata_number(manifest, "bytes", -1.0);
    const std::string expected_sha256 = metadata_string(manifest, "sha256");
    const bool bytes_passed = expected_bytes < 0.0 || static_cast<std::uint64_t>(expected_bytes) == mesh.bytes;
    const bool sha_passed = expected_sha256.empty() || expected_sha256 == mesh.sha256;
    const bool manifest_passed = file_exists && bytes_passed && sha_passed;
    checks.push_back(check_json(
        "artifact_manifest",
        manifest_passed,
        manifest_passed ? "info" : "error",
        manifest_passed ? "STL artifact exists and manifest hash/size match." : "STL artifact manifest validation failed.",
        {
            {"actualBytes", Json::number(static_cast<double>(mesh.bytes))},
            {"actualSha256", Json::string(mesh.sha256)},
            {"expectedBytes", expected_bytes < 0.0 ? Json::null() : Json::number(expected_bytes)},
            {"expectedSha256", expected_sha256.empty() ? Json::null() : Json::string(expected_sha256)},
            {"stlPath", Json::string(stl_path.string())},
        }
    ));

    checks.push_back(check_json(
        "stl_load",
        mesh.loaded,
        mesh.loaded ? "info" : "error",
        mesh.loaded ? "STL mesh loaded." : mesh.error,
        {
            {"engine", Json::string(mesh.engine)},
            {"error", mesh.error.empty() ? Json::null() : Json::string(mesh.error)},
        }
    ));

    const bool non_empty = mesh.loaded && mesh.unique_vertices > 0 && mesh.validated_triangles > 0 && mesh.bbox_volume > 0.0;
    checks.push_back(check_json(
        "mesh_non_empty",
        non_empty,
        non_empty ? "info" : "error",
        non_empty ? "Mesh has positive vertices, triangles, and bounding box volume." : "Mesh is empty or has zero bounding box volume.",
        {
            {"bbox", number_array(mesh.bbox)},
            {"bboxVolume", Json::number(mesh.bbox_volume)},
            {"triangles", Json::number(static_cast<double>(mesh.validated_triangles))},
            {"vertices", Json::number(static_cast<double>(mesh.unique_vertices))},
        }
    ));

    const bool cleanup_passed = mesh.loaded && mesh.validated_triangles > 0 && !mesh.has_degenerate_triangles;
    checks.push_back(check_json(
        "mesh_cleanup",
        cleanup_passed,
        cleanup_passed ? "info" : "error",
        cleanup_passed ? "Mesh has no index-degenerate triangles." : "Mesh contains an index-degenerate triangle.",
        {
            {"degenerateTriangles", Json::number(static_cast<double>(mesh.degenerate_triangles))},
            {"hasDegenerateTriangles", Json::boolean(mesh.has_degenerate_triangles)},
            {"rawTriangles", Json::number(static_cast<double>(mesh.raw_triangles))},
            {"validatedTriangles", Json::number(static_cast<double>(mesh.validated_triangles))},
            {"uniqueVertices", Json::number(static_cast<double>(mesh.unique_vertices))},
        }
    ));

    const bool topology_passed = mesh.loaded
        && mesh.edge_manifold_closed
        && mesh.vertex_manifold
        && mesh.orientable
        && !mesh.self_intersecting
        && mesh.has_volume;
    checks.push_back(check_json(
        "topology",
        topology_passed,
        topology_passed ? "info" : "error",
        topology_passed ? "Solid/topology checks passed." : "Solid/topology checks failed.",
        {
            {"edgeManifoldClosed", Json::boolean(mesh.edge_manifold_closed)},
            {"edgeManifoldWithBoundary", Json::boolean(mesh.edge_manifold_with_boundary)},
            {"hasVolume", Json::boolean(mesh.has_volume)},
            {"isSelfIntersecting", Json::boolean(mesh.self_intersecting)},
            {"isVertexManifold", Json::boolean(mesh.vertex_manifold)},
            {"orientable", Json::boolean(mesh.orientable)},
            {"topologyApproximate", Json::boolean(mesh.topology_approximate)},
            {"watertight", Json::boolean(mesh.watertight)},
            {"volume", Json::number(mesh.solid_volume)},
        }
    ));

    const std::array<double, 3> expected_normalized = normalize_aspect(expected);
    const std::array<double, 3> actual_normalized = normalize_aspect(mesh.bbox);
    const std::array<double, 3> deltas = {
        std::fabs(expected_normalized[0] - actual_normalized[0]),
        std::fabs(expected_normalized[1] - actual_normalized[1]),
        std::fabs(expected_normalized[2] - actual_normalized[2]),
    };
    const double max_delta = std::max({deltas[0], deltas[1], deltas[2]});
    const bool aspect_passed = mesh.loaded && max_delta <= tolerance;
    checks.push_back(check_json(
        "aspect_ratio",
        aspect_passed,
        aspect_passed ? "info" : "error",
        aspect_passed ? "Bounding box aspect ratio is within tolerance." : "Bounding box aspect ratio is outside tolerance.",
        {
            {"actual", Json::object({{"x", Json::number(mesh.bbox[0])}, {"y", Json::number(mesh.bbox[1])}, {"z", Json::number(mesh.bbox[2])}})},
            {"actualNormalized", number_array(actual_normalized)},
            {"axisDeltas", number_array(deltas)},
            {"expected", Json::object({{"tolerance", Json::number(tolerance)}, {"x", Json::number(expected[0])}, {"y", Json::number(expected[1])}, {"z", Json::number(expected[2])}})},
            {"expectedNormalized", number_array(expected_normalized)},
            {"maxAxisDelta", Json::number(max_delta)},
        }
    ));

    bool passed = true;
    for (const Json& check : checks) {
        passed = passed && get_bool(check, "passed", false);
    }

    std::map<std::string, Json> report{
        {"artifactId", Json::string(artifact_id)},
        {"checks", Json::array(checks)},
        {"contractType", Json::string("cadastrophe.structural_report.v1")},
        {"mainComponent", Json::string(main_name)},
        {"passed", Json::boolean(passed)},
        {"revisionId", Json::string(revision_id)},
        {"runId", Json::string(run_id)},
    };
    if (!passed) {
        const std::string reason = reason_for_failed_check(checks);
        report.emplace(
            "failureReport",
            Json::object({
                {"contractType", Json::string("cadastrophe.failure_report.v1")},
                {"nextAction", Json::string("refine_plan_or_source")},
                {"reason", Json::string(reason.empty() ? "structural_anchor_failed" : reason)},
            })
        );
    }
    return Json::object(std::move(report));
}
