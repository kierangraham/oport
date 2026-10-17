PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
DESTDIR ?=
CARGO ?= cargo
CARGO_FLAGS ?= --locked

.PHONY: build check test install uninstall clean

build:
	$(CARGO) build --release $(CARGO_FLAGS)

test:
	$(CARGO) test $(CARGO_FLAGS)

check:
	./tests/run

install: build
	install -Dm755 target/release/oport "$(DESTDIR)$(BINDIR)/oport"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/oport"

clean:
	$(CARGO) clean
