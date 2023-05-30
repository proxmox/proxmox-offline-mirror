include ../defines.mk

GENERATED_SYNOPSIS := 						\
	proxmox-offline-mirror/synopsis.rst			\
	proxmox-offline-mirror-helper/synopsis.rst			\
	config/mirror/config.rst

MAN1_PAGES := 				\
	proxmox-offline-mirror.1	\
	proxmox-offline-mirror-helper.1

MAN5_PAGES :=				\
	proxmox-offline-mirror.cfg.5

EXAMPLE_FILES :=	\
	examples/nginx-conf

# Sphinx documentation setup
SPHINXOPTS    =
SPHINXBUILD   = sphinx-build
BUILDDIR      = output

ifeq ($(BUILD_MODE), release)
COMPILEDIR := ../target/release
SPHINXOPTS    += -t release
else ifeq ($(BUILD_MODE), release-deb)
COMPILEDIR := ../target/$(DEB_TARGET_GNU_CPU)-unknown-$(DEB_TARGET_GNU_SYSTEM)/release
SPHINXOPTS    += -t release
else
COMPILEDIR := ../target/debug
SPHINXOPTS    += -t devbuild
endif

# Sphinx internal variables.
ALLSPHINXOPTS   = -d $(BUILDDIR)/doctrees $(SPHINXOPTS) .

all: $(MAN1_PAGES) $(MAN5_PAGES)

config/%/config.rst: $(COMPILEDIR)/docgen
	$(COMPILEDIR)/docgen $*.cfg >$@

%/synopsis.rst: $(COMPILEDIR)/%
	$< printdoc > $@

$(MAN1_PAGES) $(MAN5_PAGES): man-pages

.PHONY: man-pages
man-pages: $(GENERATED_SYNOPSIS)
	$(SPHINXBUILD) $(SPHINXOPTS) -b man ./ $(BUILDDIR)/man

.PHONY: html
html: $(GENERATED_SYNOPSIS) images/proxmox-logo.svg custom.css conf.py $(PRUNE_SIMULATOR_FILES) $(LTO_BARCODE_FILES) $(API_VIEWER_SOURCES) $(EXAMPLE_FILES)
	$(SPHINXBUILD) -b html $(ALLSPHINXOPTS) $(BUILDDIR)/html
	install -m 0644 custom.js custom.css images/proxmox-logo.svg $(BUILDDIR)/html/_static/
	@echo
	@echo "Build finished. The HTML pages are in $(BUILDDIR)/html."

.PHONY: latexpdf
latexpdf: $(GENERATED_SYNOPSIS)
	@echo "Requires python3-sphinx, texlive-xetex, xindy and texlive-fonts-extra"
	$(SPHINXBUILD) -b latex $(ALLSPHINXOPTS) $(BUILDDIR)/latex
	@echo "Running LaTeX files through xelatex..."
	$(MAKE) -C $(BUILDDIR)/latex all-pdf
	@echo "xelatex finished; the PDF files are in $(BUILDDIR)/latex."

.PHONY: epub3
epub3: $(GENERATED_SYNOPSIS)
	$(SPHINXBUILD) -b epub3 $(ALLSPHINXOPTS) $(BUILDDIR)/epub3
	@echo
	@echo "Build finished. The epub3 file is in $(BUILDDIR)/epub3."

clean:
	rm -r -f *~ *.1 $(BUILDDIR) $(GENERATED_SYNOPSIS)

install_manual_pages: man-pages
	install -dm755 $(DESTDIR)$(MAN1DIR)
	for i in $(MAN1_PAGES); do install -m755 $(BUILDDIR)/man/$$i $(DESTDIR)$(MAN1DIR)/ ; done
	install -dm755 $(DESTDIR)$(MAN5DIR)
	for i in $(MAN5_PAGES); do install -m755 $(BUILDDIR)/man/$$i $(DESTDIR)$(MAN5DIR)/ ; done

install_html: html
	install -dm755 $(DESTDIR)$(DOCDIR)
	rsync -a $(BUILDDIR)/html $(DESTDIR)$(DOCDIR)

install_pdf: latexpdf
	install -dm755 $(DESTDIR)$(DOCDIR)
	install -m 0644 output/latex/ProxmoxOfflineMirror.pdf $(DESTDIR)$(DOCDIR)/proxmox-offline-mirror.pdf

install_examples: $(EXAMPLE_FILES)
	install -dm755 $(DESTDIR)$(DOCDIR)/examples
	install -m 0644 examples/nginx-conf $(DESTDIR)$(DOCDIR)/examples

ifneq ($(filter nodoc,$(DEB_BUILD_PROFILES)),)

install: install_manual_pages

else

install: install_manual_pages install_html install_pdf install_examples

endif
