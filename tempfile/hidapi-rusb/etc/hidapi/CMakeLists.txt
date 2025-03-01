cmake_minimum_required(VERSION 3.1.3 FATAL_ERROR)

if(NOT CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    add_subdirectory(src)
    # compatinility with find_package() vs add_subdirectory
    set(hidapi_VERSION "${hidapi_VERSION}" PARENT_SCOPE)
    return()
endif()
# All of the below in this file is meant for a standalone build.
# When building as a subdirectory of a larger project, most of the options may not make sense for it,
# so it is up to developer to configure those, e.g.:
#
# # a subfolder of a master project, e.g.: 3rdparty/hidapi/CMakeLists.txt
#
# set(HIDAPI_WITH_HIDRAW OFF)
# set(CMAKE_FRAMEWORK ON)
# # and keep everything else to their defaults
# add_subdirectory(hidapi)
#

set(DEFAULT_CMAKE_BUILD_TYPES "Debug" "Release" "MinSizeRel" "RelWithDebInfo")
if(NOT DEFINED CMAKE_BUILD_TYPE OR NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE "Release" CACHE STRING "${DEFAULT_CMAKE_BUILD_TYPES}" FORCE)
endif()
# This part is for convenience, when used one of the standard build types with cmake-gui
list(FIND DEFAULT_CMAKE_BUILD_TYPES "${CMAKE_BUILD_TYPE}" _build_type_index)
if(${_build_type_index} GREATER -1)
    # set it optionally, so a custom CMAKE_BUILD_TYPE can be used as well, if needed
    set_property(CACHE CMAKE_BUILD_TYPE PROPERTY STRINGS ${DEFAULT_CMAKE_BUILD_TYPES})
endif()
unset(_build_type_index)
#

project(hidapi LANGUAGES C)

if(APPLE)
    if(NOT CMAKE_VERSION VERSION_LESS "3.15")
        option(CMAKE_FRAMEWORK "Build macOS/iOS Framework version of the library" OFF)
    endif()
elseif(NOT WIN32)
    if(CMAKE_SYSTEM_NAME MATCHES "Linux")
        option(HIDAPI_WITH_HIDRAW "Build HIDRAW-based implementation of HIDAPI" ON)
        option(HIDAPI_WITH_LIBUSB "Build LIBUSB-based implementation of HIDAPI" ON)
    endif()
endif()

option(BUILD_SHARED_LIBS "Build shared version of the libraries, otherwise build statically" ON)

set(HIDAPI_INSTALL_TARGETS ON)
set(HIDAPI_PRINT_VERSION ON)

add_subdirectory(src)

set(BUILD_HIDTEST_DEFAULT OFF)
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(BUILD_HIDTEST_DEFAULT ON)
endif()
option(HIDAPI_BUILD_HIDTEST "Build small console test application hidtest" ${BUILD_HIDTEST_DEFAULT})
if(HIDAPI_BUILD_HIDTEST)
    add_subdirectory(hidtest)
endif()
