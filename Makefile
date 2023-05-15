VERSION=$(shell grep ^version Cargo.toml|cut -d\" -f2)

all:
	@echo "Select target"

tag:
	git tag -a v${VERSION} -m v${VERSION}
	git push origin --tags

release: pub tag
