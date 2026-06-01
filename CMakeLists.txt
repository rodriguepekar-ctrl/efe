cmake_minimum_required(VERSION 3.28)
project(efe_kernels
    VERSION 0.0.1
    DESCRIPTION "EFE CUDA kernels"
    LANGUAGES CUDA CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CUDA_STANDARD 17)
set(CMAKE_CUDA_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# Default to H100 (compute capability 90). Override via CMAKE_CUDA_ARCHITECTURES.
if(NOT DEFINED CMAKE_CUDA_ARCHITECTURES)
    set(CMAKE_CUDA_ARCHITECTURES 90)
endif()

# Detect CUDA
find_package(CUDAToolkit 12.4 REQUIRED)

# Sources (stubs for Sprint 0; real kernels arrive Sprint 1+)
set(EFE_KERNEL_SOURCES
    crates/efe-kernels/cuda/stub.cu
)

add_library(efe_kernels STATIC ${EFE_KERNEL_SOURCES})

target_include_directories(efe_kernels PUBLIC
    $<BUILD_INTERFACE:${CMAKE_SOURCE_DIR}/crates/efe-kernels/cuda/include>
)

target_compile_options(efe_kernels PRIVATE
    $<$<COMPILE_LANGUAGE:CUDA>:
        --use_fast_math
        -Xcompiler=-fPIC
        -O3
        --expt-relaxed-constexpr
        -Werror=cross-execution-space-call
    >)

target_link_libraries(efe_kernels PUBLIC CUDA::cudart)

# Version info via compile defines
target_compile_definitions(efe_kernels PRIVATE
    EFE_VERSION_MAJOR=${PROJECT_VERSION_MAJOR}
    EFE_VERSION_MINOR=${PROJECT_VERSION_MINOR}
    EFE_VERSION_PATCH=${PROJECT_VERSION_PATCH}
)

# Tests
enable_testing()
if(EXISTS ${CMAKE_SOURCE_DIR}/tests/cuda/CMakeLists.txt)
    add_subdirectory(tests/cuda)
endif()

message(STATUS "efe_kernels configured: CUDA ${CUDAToolkit_VERSION}, archs ${CMAKE_CUDA_ARCHITECTURES}")
