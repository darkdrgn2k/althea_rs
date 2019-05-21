#!/bin/bash
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Parse command line arguments
source $DIR/build_common.sh

if [[ ! -d $DIR/staging_dir ]]; then
    pushd $DIR
    wget -N https://updates.altheamesh.com/staging.tar.xz -O staging.tar.xz > /dev/null; tar -xf staging.tar.xz
fi

export TOOLCHAIN=toolchain-arm_cortex-a7+neon-vfpv4_gcc-7.3.0_musl_eabi
export TARGET_CC=$DIR/staging_dir/$TOOLCHAIN/bin/arm-openwrt-linux-muslgnueabi-gcc
export TARGET_LD=$DIR/staging_dir/$TOOLCHAIN/bin/arm-openwrt-linux-muslgnueabi-ld
export TARGET_AR=$DIR/staging_dir/$TOOLCHAIN/bin/arm-openwrt-linux-muslgnueabi-ar
export CARGO_TARGET_ARM_UNKNOWN_LINUX_MUSL_LINKER=$TARGET_CC
export CARGO_TARGET_ARM_UNKNOWN_LINUX_MUSL_AR=$TARGET_AR
export SQLITE3_LIB_DIR=$DIR/staging_dir/target-arm_cortex-a7+neon-vfpv4_musl_eabi/usr/lib/
export MIPS_UNKNOWN_LINUX_MUSL_OPENSSL_DIR=$DIR/staging_dir/target-arm_cortex-a7+neon-vfpv4_musl_eabi/usr/lib/
export OPENSSL_STATIC=1
export PKG_CONFIG_ALLOW_CROSS=1
export OPENSSL_DIR=$DIR/staging_dir/target-arm_cortex-a7+neon-vfpv4_musl_eabi/usr/

rustup target add arm-unknown-linux-gnueabihf

cargo build --target arm-unknown-linux-gnueabihf ${PROFILE} ${FEATURES} -p rita --bin rita
