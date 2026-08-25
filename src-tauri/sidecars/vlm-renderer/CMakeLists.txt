cmake_minimum_required(VERSION 3.20)

project(cadgen_ax_vlm_renderer LANGUAGES CXX)

add_executable(cadgen-ax-vlm-renderer src/main.cpp)
target_compile_features(cadgen-ax-vlm-renderer PRIVATE cxx_std_17)
