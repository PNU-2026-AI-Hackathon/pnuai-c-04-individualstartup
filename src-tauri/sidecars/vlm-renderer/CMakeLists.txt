cmake_minimum_required(VERSION 3.20)

project(cadastrophe_vlm_renderer LANGUAGES CXX)

add_executable(cadastrophe-vlm-renderer src/main.cpp)
target_compile_features(cadastrophe-vlm-renderer PRIVATE cxx_std_17)
