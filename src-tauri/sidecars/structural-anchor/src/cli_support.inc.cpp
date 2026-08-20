std::string read_stdin() {
    std::ostringstream buffer;
    buffer << std::cin.rdbuf();
    return buffer.str();
}

void print_help() {
    std::cout
        << "cadastrophe-structural-anchor\n"
        << "Usage: cadastrophe-structural-anchor [--input report-input.json] [--pretty]\n"
        << "       cadastrophe-structural-anchor --input-stl model.stl [--pretty]\n\n"
        << "Development shortcut: --input-stl validates one STL and prints only mesh results.\n\n"
        << "Input JSON fields: runId, revisionId, artifactId, plan|planPath, stlPath,\n"
        << "artifactManifest|artifactManifestPath, runtimeDiagnostics|runtimeDiagnosticsPath,\n"
        << "and sourceText|sourcePath or manifest metadata for the @main_component check.\n";
}
