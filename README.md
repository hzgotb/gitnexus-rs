新克隆项目时，应执行以下指令：
```sh
git clone --recurse-submodules git@gitcode.com:hzgotb/gitnexus-ee.git
```

已经克隆的话，执行以下指令：
```sh
git submodule init && git submodule update
```


更新子模块
```sh
git submodule update--remote
```

构建 zig
```sh
./scripts/build-zig.sh
./scripts/build-zig.sh test-cli
```
