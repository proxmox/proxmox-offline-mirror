include /usr/share/dpkg/pkg-info.mk
include /usr/share/dpkg/architecture.mk
include defines.mk

PACKAGE=proxmox-offline-mirror
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)

SUBDIRS := docs

DEB=$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_HOST_ARCH).deb
HELPER_DEB=$(PACKAGE)-helper_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_HOST_ARCH).deb
DBG_DEB=$(PACKAGE)-dbgsym_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_HOST_ARCH).deb
HELPER_DBG_DEB=$(PACKAGE)-helper-dbgsym_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_HOST_ARCH).deb
DOC_DEB=$(PACKAGE)-docs_$(DEB_VERSION_UPSTREAM_REVISION)_all.deb
DSC=rust-$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION).dsc

DEBS = $(DEB) $(HELPER_DEB) $(DBG_DEB) $(HELPER_DBG_DEB) $(DOC_DEB)

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

USR_BIN := \
	proxmox-offline-mirror \
	proxmox-offline-mirror-helper

COMPILED_BINS := \
	$(addprefix $(COMPILEDIR)/,$(USR_BIN))

all: cargo-build $(SUBDIRS)

.PHONY: cargo-build
cargo-build:
	cargo build $(CARGO_BUILD_ARGS)

.PHONY: $(SUBDIRS)
$(SUBDIRS): cargo-build
	$(MAKE) -C $@

$(COMPILED_BINS): cargo-build

install: $(COMPILED_BINS)
	$(MAKE) -C docs install DESTDIR=../debian/proxmox-offline-mirror-docs
	install -dm755 $(DESTDIR)$(BINDIR)
	$(foreach i,$(USR_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/ ;)

update-dcontrol: #$(BUILDDIR)
	debcargo package \
	  --config debian/debcargo.toml \
	  --changelog-ready \
	  --no-overlay-write-back \
	  --directory $(BUILDDIR) \
	  $(PACKAGE) \
	  $(shell dpkg-parsechangelog -l debian/changelog -SVersion | sed -e 's/-.*//')
	cat $(BUILDDIR)/debian/control debian/control.extra > debian/control
	wrap-and-sort -t -k -f debian/control

.PHONY: build
build: $(BUILDDIR)
$(BUILDDIR):
	rm -rf $@ $@.tmp; mkdir $@.tmp
	cp -a src docs debian Cargo.toml Makefile defines.mk $@.tmp/
	mv $@.tmp $@

.PHONY: deb
deb: $(DEB)
$(HELPER_DEB) $(DBG_DEB) $(HELPER_DBG_DEB) $(DOC_DEB): $(DEB)
$(DEB): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEB) $(DOC_DEB) $(HELPER_DEB)

.PHONY: dsc
dsc:
	rm -rf $(BUILDDIR) $(DSC)
	$(MAKE) $(DSC)
	lintian $(DSC)
$(DSC): $(BUILDDIR)
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d -nc

sbuild: $(DSC)
	sbuild $<

.PHONY: dinstall
dinstall: $(DEB)
	dpkg -i $(DEB) $(DBG_DEB) $(DOC_DEB)

.PHONY: upload
upload: UPLOAD_DIST ?= $(DEB_DISTRIBUTION)
upload: $(DEBS)
	tar cf - $(DEBS) | ssh -X repoman@repo.proxmox.com -- upload --product pve,pmg,pbs,pbs-client --dist $(UPLOAD_DIST) --arch $(DEB_HOST_ARCH)

.PHONY: distclean
distclean: clean

.PHONY: clean
clean:
	cargo clean
	rm -f *.deb *.build *.buildinfo *.changes *.dsc rust-$(PACKAGE)*.tar*
	rm -rf $(PACKAGE)-[0-9]*/
	find . -name '*~' -exec rm {} ';'
