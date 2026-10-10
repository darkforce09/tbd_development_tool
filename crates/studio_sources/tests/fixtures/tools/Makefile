.PHONY: build test

VERSION := 1.0

# Build everything
build: deps
	cargo build

test: ## Run the tests
	cargo test

deps:
	true

%.o: %.c
	cc -c $<

clean dist: ## Remove outputs
	rm -rf target
