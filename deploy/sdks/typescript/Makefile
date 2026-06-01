.PHONY: install gen build test clean

install:
	npm install

gen:
	bash ../../proto-gen/scripts/gen-ts.sh

build: gen
	npm run build

test:
	npm test

clean:
	rm -rf node_modules dist src/generated
