include /usr/share/dpkg/pkg-info.mk
include /usr/share/dpkg/architecture.mk
include defines.mk

PACKAGE=proxmox-offline-mirror
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)
BUILDDIR_TMP ?= $(BUILDDIR).tmp

SUBDIRS := docs

DEB=$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
HELPER_DEB=$(PACKAGE)-helper_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
LIB_DEB=librust-$(PACKAGE)-dev_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
DBG_DEB=$(PACKAGE)-dbgsym_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
HELPER_DBG_DEB=$(PACKAGE)-helper-dbgsym_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
DOC_DEB=$(PACKAGE)-docs_$(DEB_VERSION_UPSTREAM_REVISION)_all.deb
DSC=rust-$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION).dsc

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

all: cargo-build $(SUBDIRS)

.PHONY: cargo-build
cargo-build:
	cargo build $(CARGO_BUILD_ARGS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

.PHONY: build
build:
	rm -rf $(BUILDDIR) $(BUILDDIR_TMP); mkdir $(BUILDDIR_TMP)
	rm -f debian/control
	debcargo package \
	  --config debian/debcargo.toml \
	  --changelog-ready \
	  --no-overlay-write-back \
	  --directory $(BUILDDIR_TMP) \
	  $(PACKAGE) \
	  $(shell dpkg-parsechangelog -l debian/changelog -SVersion | sed -e 's/-.*//')
	cat $(BUILDDIR_TMP)/debian/control debian/control.extra > debian/control
	cp -a debian/control $(BUILDDIR_TMP)/debian/control
	rm -f $(BUILDDIR_TMP)/Cargo.lock
	find $(BUILDDIR_TMP)/debian -name "*.hint" -delete
	mv $(BUILDDIR_TMP) $(BUILDDIR)

.PHONY: deb
deb: $(DEB)
$(DEB): build
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEB) $(DOC_DEB) $(HELPER_DEB)

.PHONY: dsc
dsc: $(DSC)
$(DSC): build
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

.PHONY: dinstall
dinstall: $(DEB)
	dpkg -i $(DEB) $(DBG_DEB) $(DOC_DEB)

.PHONY: upload
upload: $(DEB)
	tar cf - $(DEB) $(HELPER_DEB) $(DBG_DEB) $(HELPER_DBG_DEB) $(DOC_DEB) | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs,pbs-client --dist bullseye --arch $(DEB_BUILD_ARCH)

.PHONY: distclean
distclean: clean

.PHONY: clean
clean:
	cargo clean
	rm -rf *.deb *.buildinfo *.changes *.dsc rust-$(PACKAGE)_*.tar.?z $(PACKAGE)-*/
	find . -name '*~' -exec rm {} ';'
