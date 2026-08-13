cmake_minimum_required(VERSION 3.20)

project(cadastrophe_structural_anchor LANGUAGES CXX)

add_library(cadastrophe-mesh-validator STATIC src/mesh_validator.cpp)
target_compile_features(cadastrophe-mesh-validator PUBLIC cxx_std_17)
target_include_directories(cadastrophe-mesh-validator PUBLIC src)

add_executable(cadastrophe-structural-anchor src/main.cpp)
target_compile_features(cadastrophe-structural-anchor PRIVATE cxx_std_17)
target_link_libraries(cadastrophe-structural-anchor PRIVATE cadastrophe-mesh-validator)
