std::string read_stdin() {
    std::ostringstream buffer;
    buffer << std::cin.rdbuf();
    return buffer.str();
}

void print_help() {
    std::cout
        << "cadastrophe-structural-anchor\n"
        << "Usage: cadastrophe-structural-anchor [--input report-input.json] [--pretty]\n\n"
        << "Input JSON fields: runId, revisionId, artifactId, plan|planPath, stlPath,\n"
        << "artifactManifest|artifactManifestPath, runtimeDiagnostics|runtimeDiagnosticsPath,\n"
        << "and sourceText|sourcePath or manifest metadata for the @main_component check.\n";
}
