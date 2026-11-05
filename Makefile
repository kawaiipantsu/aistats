.PHONY: build install test deb clean

build:
	cargo build --release --locked

install:
	cargo install --path . --locked

test:
	cargo test --locked
	python3 -B -m unittest discover -s src/fun -p remote_test.py

deb:
	./contrib/build-deb.sh $(ARCH)

clean:
	cargo clean
