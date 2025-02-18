# lcpc

Polynomial commitment scheme from any linear code with sufficient minimum distance.

https://eprint.iacr.org/2021/1043

# repo layout

- `doc` - misc documentation

- `lcpc-2d` - "2-dimensional" polynomial commitment parameterized by a linear code

- `lcpc-brakedown-pc` - an expander-based encoding for use with `lcpc-2d`

- `lcpc-ligero-pc` - an R-S--based encoding for use with `lcpc-2d`

- `lcpc-test-fields` - field definitions and misc for testing / benching

- `proof-of-storage` - a showcase of how this can be used for proof of remote file retrievability

- `scripts` - miscellaneous

## license

    Copyright 2021 Riad S. Wahby and lcpc authors

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.

