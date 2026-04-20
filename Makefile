TARGET ?= x86_64-unknown-redox
LINKER ?= $(shell redoxer env which $(shell redoxer env printenv LD))
BOARD ?=
BUILD_TYPE ?= release
BUILD_FLAGS ?= --release
CARGO ?= redoxer
CARGO_HOST ?= env -u CARGO -u RUSTFLAGS cargo

SRC_DIR ?= $(CURDIR)
BUILD_DIR ?= $(shell pwd)/target/$(TARGET)/build
DESTDIR ?= ./sysroot
SYSROOT ?= $(shell pwd)/target/$(TARGET)/sysroot
export REDOXER_SYSROOT=$(SYSROOT)
TARGET_DIR = $(BUILD_DIR)/$(TARGET)/$(BUILD_TYPE)
BUILD_FLAGS +=  --target-dir $(BUILD_DIR)

INITFS_BINS = init logd ramfs randd zerod \
	acpid fbbootlogd fbcond hwd inputd lived \
	pcid pcid-spawner rtcd vesad
INITFS_DRIVERS_BINS = nvmed virtio-blkd  virtio-gpud
BASE_BINS = inputd pcid pcid-spawner redoxerd audiod dhcpd ipcd ptyd netstack
DRIVERS_BINS = e1000d ihdad ihdgd ixgbed rtl8139d rtl8168d \
	usbctl usbhidd usbhubd usbscsid virtio-netd xhcid

ifneq (,$(filter i586-unknown-redox i686-unknown-redox x86_64-unknown-redox,$(TARGET)))
    INITFS_BINS += ps2d
    INITFS_DRIVERS_BINS += ahcid ided
    DRIVERS_BINS += ac97d sb16d vboxd
endif

ifeq ($(TARGET),aarch64-unknown-redox)
    ifeq ($(BOARD),raspi3b)
        INITFS_BINS += bcm2835-sdhcid
    endif
endif

INITFS_CARGO_ARGS = $(foreach bin,$(INITFS_BINS),-p $(bin))
INITFS_DRIVERS_CARGO_ARGS = $(foreach bin,$(INITFS_DRIVERS_BINS),-p $(bin))
BASE_CARGO_ARGS = $(foreach bin,$(BASE_BINS),-p $(bin))
DRIVERS_CARGO_ARGS = $(foreach bin,$(DRIVERS_BINS),-p $(bin))

.PHONY: all initfs base install install-initfs install-base test

all: initfs base
install: install-initfs install-base

clean:
	rm -rf $(SRC_DIR)/target $(SRC_DIR)/sysroot $(SYSROOT) $(TARGET_DIR)

# test if booting
test: all
	$(MAKE) install
	redoxer exec --folder ./sysroot/:/ true

# test with interactive gui
test-gui: all
	$(MAKE) install
	redoxer exec --gui --folder ./sysroot/:/ ion

# -----------------------------------------------------------------------------
# base-initfs
# -----------------------------------------------------------------------------
$(SYSROOT)/bin/redoxfs:
	redoxer pkg redoxfs

initfs: $(SYSROOT)/bin/redoxfs
	mkdir -pv "$(BUILD_DIR)"
	rm -rf "$(BUILD_DIR)/initfs"
# Copy config files
	mkdir -p "$(BUILD_DIR)/initfs/lib/init.d" "$(BUILD_DIR)/initfs/lib/pcid.d"
	cp "$(SRC_DIR)/init.initfs.d"/* "$(BUILD_DIR)/initfs/lib/init.d/"
	cp "$(SRC_DIR)/drivers/initfs.toml" "$(BUILD_DIR)/initfs/lib/pcid.d/initfs.toml"
# Build daemons and drivers
	CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_PANIC=abort \
		$(CARGO) build $(BUILD_FLAGS) \
		--manifest-path "$(SRC_DIR)/Cargo.toml" \
		$(INITFS_CARGO_ARGS) $(INITFS_DRIVERS_CARGO_ARGS)
# Distribute binaries
	mkdir -pv "$(BUILD_DIR)/initfs/bin" "$(BUILD_DIR)/initfs/lib/drivers"
	for bin in $(INITFS_BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(BUILD_DIR)/initfs/bin"; \
	done
	for bin in $(INITFS_DRIVERS_BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(BUILD_DIR)/initfs/lib/drivers"; \
	done
	cp "$(SYSROOT)/bin/redoxfs" "$(BUILD_DIR)/initfs/bin"

	cd "$(SRC_DIR)/bootstrap" && $(CARGO) rustc $(BUILD_FLAGS) \
		-- -Ctarget-feature=+crt-static -Clinker="$(LINKER)"

	$(CARGO_HOST) run --manifest-path "$(SRC_DIR)/initfs/tools/Cargo.toml" --bin redox-initfs-ar -- \
		"$(BUILD_DIR)/initfs" "$(TARGET_DIR)/bootstrap" -o "$(BUILD_DIR)/initfs.img"

install-initfs: initfs
	@mkdir -pv "$(DESTDIR)/usr/lib/boot"
	@cp -v "$(BUILD_DIR)/initfs.img" "$(DESTDIR)/usr/lib/boot/initfs"

# -----------------------------------------------------------------------------
# base
# -----------------------------------------------------------------------------
base:
# Build daemons and drivers
	CARGO_PROFILE_RELEASE_OPT_LEVEL=s CARGO_PROFILE_RELEASE_PANIC=abort \
		$(CARGO) build $(BUILD_FLAGS) \
		--manifest-path "$(SRC_DIR)/Cargo.toml" \
		$(BASE_CARGO_ARGS) $(DRIVERS_CARGO_ARGS)

install-base: base
	@mkdir -pv "$(DESTDIR)/usr/bin" "$(DESTDIR)/usr/lib/drivers"
	@mkdir -pv "$(DESTDIR)/usr/lib/init.d/" "$(DESTDIR)/usr/lib/pcid.d"
# Distribute binaries
	@for bin in $(BASE_BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(DESTDIR)/usr/bin"; \
	done
	@for bin in $(DRIVERS_BINS); do \
		cp -v "$(TARGET_DIR)/$$bin" "$(DESTDIR)/usr/lib/drivers"; \
	done
# Copy configurations
	@cp -v "$(SRC_DIR)/init.d"/* "$(DESTDIR)/usr/lib/init.d/"
	@find "$(SRC_DIR)/drivers" -maxdepth 3 -type f -name 'config.toml' | while read -r conf; do \
		driver=$$(basename "$$(dirname "$$conf")"); \
		cp -v "$$conf" "$(DESTDIR)/usr/lib/pcid.d/$$driver.toml"; \
	done
