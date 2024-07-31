[
    // `log(int)` -> `log(int256)`
    // `4e0c1d1d` -> `2d5b6cb9`
    ([78, 12, 29, 29], [45, 91, 108, 185]),
    // `log(uint)` -> `log(uint256)`
    // `f5b1bba9` -> `f82c50f1`
    ([245, 177, 187, 169], [248, 44, 80, 241]),
    // `log(uint)` -> `log(uint256)`
    // `f5b1bba9` -> `f82c50f1`
    ([245, 177, 187, 169], [248, 44, 80, 241]),
    // `log(int)` -> `log(int256)`
    // `4e0c1d1d` -> `2d5b6cb9`
    ([78, 12, 29, 29], [45, 91, 108, 185]),
    // `log(uint,uint)` -> `log(uint256,uint256)`
    // `6c0f6980` -> `f666715a`
    ([108, 15, 105, 128], [246, 102, 113, 90]),
    // `log(uint,string)` -> `log(uint256,string)`
    // `0fa3f345` -> `643fd0df`
    ([15, 163, 243, 69], [100, 63, 208, 223]),
    // `log(uint,bool)` -> `log(uint256,bool)`
    // `1e6dd4ec` -> `1c9d7eb3`
    ([30, 109, 212, 236], [28, 157, 126, 179]),
    // `log(uint,address)` -> `log(uint256,address)`
    // `58eb860c` -> `69276c86`
    ([88, 235, 134, 12], [105, 39, 108, 134]),
    // `log(string,uint)` -> `log(string,uint256)`
    // `9710a9d0` -> `b60e72cc`
    ([151, 16, 169, 208], [182, 14, 114, 204]),
    // `log(string,int)` -> `log(string,int256)`
    // `af7faa38` -> `3ca6268e`
    ([175, 127, 170, 56], [60, 166, 38, 142]),
    // `log(bool,uint)` -> `log(bool,uint256)`
    // `364b6a92` -> `399174d3`
    ([54, 75, 106, 146], [57, 145, 116, 211]),
    // `log(address,uint)` -> `log(address,uint256)`
    // `2243cfa3` -> `8309e8a8`
    ([34, 67, 207, 163], [131, 9, 232, 168]),
    // `log(uint,uint,uint)` -> `log(uint256,uint256,uint256)`
    // `e7820a74` -> `d1ed7a3c`
    ([231, 130, 10, 116], [209, 237, 122, 60]),
    // `log(uint,uint,string)` -> `log(uint256,uint256,string)`
    // `7d690ee6` -> `71d04af2`
    ([125, 105, 14, 230], [113, 208, 74, 242]),
    // `log(uint,uint,bool)` -> `log(uint256,uint256,bool)`
    // `67570ff7` -> `4766da72`
    ([103, 87, 15, 247], [71, 102, 218, 114]),
    // `log(uint,uint,address)` -> `log(uint256,uint256,address)`
    // `be33491b` -> `5c96b331`
    ([190, 51, 73, 27], [92, 150, 179, 49]),
    // `log(uint,string,uint)` -> `log(uint256,string,uint256)`
    // `5b6de83f` -> `37aa7d4c`
    ([91, 109, 232, 63], [55, 170, 125, 76]),
    // `log(uint,string,string)` -> `log(uint256,string,string)`
    // `3f57c295` -> `b115611f`
    ([63, 87, 194, 149], [177, 21, 97, 31]),
    // `log(uint,string,bool)` -> `log(uint256,string,bool)`
    // `46a7d0ce` -> `4ceda75a`
    ([70, 167, 208, 206], [76, 237, 167, 90]),
    // `log(uint,string,address)` -> `log(uint256,string,address)`
    // `1f90f24a` -> `7afac959`
    ([31, 144, 242, 74], [122, 250, 201, 89]),
    // `log(uint,bool,uint)` -> `log(uint256,bool,uint256)`
    // `5a4d9922` -> `20098014`
    ([90, 77, 153, 34], [32, 9, 128, 20]),
    // `log(uint,bool,string)` -> `log(uint256,bool,string)`
    // `8b0e14fe` -> `85775021`
    ([139, 14, 20, 254], [133, 119, 80, 33]),
    // `log(uint,bool,bool)` -> `log(uint256,bool,bool)`
    // `d5ceace0` -> `20718650`
    ([213, 206, 172, 224], [32, 113, 134, 80]),
    // `log(uint,bool,address)` -> `log(uint256,bool,address)`
    // `424effbf` -> `35085f7b`
    ([66, 78, 255, 191], [53, 8, 95, 123]),
    // `log(uint,address,uint)` -> `log(uint256,address,uint256)`
    // `884343aa` -> `5a9b5ed5`
    ([136, 67, 67, 170], [90, 155, 94, 213]),
    // `log(uint,address,string)` -> `log(uint256,address,string)`
    // `ce83047b` -> `63cb41f9`
    ([206, 131, 4, 123], [99, 203, 65, 249]),
    // `log(uint,address,bool)` -> `log(uint256,address,bool)`
    // `7ad0128e` -> `9b6ec042`
    ([122, 208, 18, 142], [155, 110, 192, 66]),
    // `log(uint,address,address)` -> `log(uint256,address,address)`
    // `7d77a61b` -> `bcfd9be0`
    ([125, 119, 166, 27], [188, 253, 155, 224]),
    // `log(string,uint,uint)` -> `log(string,uint256,uint256)`
    // `969cdd03` -> `ca47c4eb`
    ([150, 156, 221, 3], [202, 71, 196, 235]),
    // `log(string,uint,string)` -> `log(string,uint256,string)`
    // `a3f5c739` -> `5970e089`
    ([163, 245, 199, 57], [89, 112, 224, 137]),
    // `log(string,uint,bool)` -> `log(string,uint256,bool)`
    // `f102ee05` -> `ca7733b1`
    ([241, 2, 238, 5], [202, 119, 51, 177]),
    // `log(string,uint,address)` -> `log(string,uint256,address)`
    // `e3849f79` -> `1c7ec448`
    ([227, 132, 159, 121], [28, 126, 196, 72]),
    // `log(string,string,uint)` -> `log(string,string,uint256)`
    // `f362ca59` -> `5821efa1`
    ([243, 98, 202, 89], [88, 33, 239, 161]),
    // `log(string,bool,uint)` -> `log(string,bool,uint256)`
    // `291bb9d0` -> `c95958d6`
    ([41, 27, 185, 208], [201, 89, 88, 214]),
    // `log(string,address,uint)` -> `log(string,address,uint256)`
    // `07c81217` -> `0d26b925`
    ([7, 200, 18, 23], [13, 38, 185, 37]),
    // `log(bool,uint,uint)` -> `log(bool,uint256,uint256)`
    // `3b5c03e0` -> `37103367`
    ([59, 92, 3, 224], [55, 16, 51, 103]),
    // `log(bool,uint,string)` -> `log(bool,uint256,string)`
    // `c8397eb0` -> `c3fc3970`
    ([200, 57, 126, 176], [195, 252, 57, 112]),
    // `log(bool,uint,bool)` -> `log(bool,uint256,bool)`
    // `1badc9eb` -> `e8defba9`
    ([27, 173, 201, 235], [232, 222, 251, 169]),
    // `log(bool,uint,address)` -> `log(bool,uint256,address)`
    // `c4d23507` -> `088ef9d2`
    ([196, 210, 53, 7], [8, 142, 249, 210]),
    // `log(bool,string,uint)` -> `log(bool,string,uint256)`
    // `c0382aac` -> `1093ee11`
    ([192, 56, 42, 172], [16, 147, 238, 17]),
    // `log(bool,bool,uint)` -> `log(bool,bool,uint256)`
    // `b01365bb` -> `12f21602`
    ([176, 19, 101, 187], [18, 242, 22, 2]),
    // `log(bool,address,uint)` -> `log(bool,address,uint256)`
    // `eb704baf` -> `5f7b9afb`
    ([235, 112, 75, 175], [95, 123, 154, 251]),
    // `log(address,uint,uint)` -> `log(address,uint256,uint256)`
    // `8786135e` -> `b69bcaf6`
    ([135, 134, 19, 94], [182, 155, 202, 246]),
    // `log(address,uint,string)` -> `log(address,uint256,string)`
    // `baf96849` -> `a1f2e8aa`
    ([186, 249, 104, 73], [161, 242, 232, 170]),
    // `log(address,uint,bool)` -> `log(address,uint256,bool)`
    // `e54ae144` -> `678209a8`
    ([229, 74, 225, 68], [103, 130, 9, 168]),
    // `log(address,uint,address)` -> `log(address,uint256,address)`
    // `97eca394` -> `7bc0d848`
    ([151, 236, 163, 148], [123, 192, 216, 72]),
    // `log(address,string,uint)` -> `log(address,string,uint256)`
    // `1cdaf28a` -> `67dd6ff1`
    ([28, 218, 242, 138], [103, 221, 111, 241]),
    // `log(address,bool,uint)` -> `log(address,bool,uint256)`
    // `2c468d15` -> `9c4f99fb`
    ([44, 70, 141, 21], [156, 79, 153, 251]),
    // `log(address,address,uint)` -> `log(address,address,uint256)`
    // `6c366d72` -> `17fe6185`
    ([108, 54, 109, 114], [23, 254, 97, 133]),
    // `log(uint,uint,uint,uint)` -> `log(uint256,uint256,uint256,uint256)`
    // `5ca0ad3e` -> `193fb800`
    ([92, 160, 173, 62], [25, 63, 184, 0]),
    // `log(uint,uint,uint,string)` -> `log(uint256,uint256,uint256,string)`
    // `78ad7a0c` -> `59cfcbe3`
    ([120, 173, 122, 12], [89, 207, 203, 227]),
    // `log(uint,uint,uint,bool)` -> `log(uint256,uint256,uint256,bool)`
    // `6452b9cb` -> `c598d185`
    ([100, 82, 185, 203], [197, 152, 209, 133]),
    // `log(uint,uint,uint,address)` -> `log(uint256,uint256,uint256,address)`
    // `e0853f69` -> `fa8185af`
    ([224, 133, 63, 105], [250, 129, 133, 175]),
    // `log(uint,uint,string,uint)` -> `log(uint256,uint256,string,uint256)`
    // `3894163d` -> `5da297eb`
    ([56, 148, 22, 61], [93, 162, 151, 235]),
    // `log(uint,uint,string,string)` -> `log(uint256,uint256,string,string)`
    // `7c032a32` -> `27d8afd2`
    ([124, 3, 42, 50], [39, 216, 175, 210]),
    // `log(uint,uint,string,bool)` -> `log(uint256,uint256,string,bool)`
    // `b22eaf06` -> `7af6ab25`
    ([178, 46, 175, 6], [122, 246, 171, 37]),
    // `log(uint,uint,string,address)` -> `log(uint256,uint256,string,address)`
    // `433285a2` -> `42d21db7`
    ([67, 50, 133, 162], [66, 210, 29, 183]),
    // `log(uint,uint,bool,uint)` -> `log(uint256,uint256,bool,uint256)`
    // `6c647c8c` -> `eb7f6fd2`
    ([108, 100, 124, 140], [235, 127, 111, 210]),
    // `log(uint,uint,bool,string)` -> `log(uint256,uint256,bool,string)`
    // `efd9cbee` -> `a5b4fc99`
    ([239, 217, 203, 238], [165, 180, 252, 153]),
    // `log(uint,uint,bool,bool)` -> `log(uint256,uint256,bool,bool)`
    // `94be3bb1` -> `ab085ae6`
    ([148, 190, 59, 177], [171, 8, 90, 230]),
    // `log(uint,uint,bool,address)` -> `log(uint256,uint256,bool,address)`
    // `e117744f` -> `9a816a83`
    ([225, 23, 116, 79], [154, 129, 106, 131]),
    // `log(uint,uint,address,uint)` -> `log(uint256,uint256,address,uint256)`
    // `610ba8c0` -> `88f6e4b2`
    ([97, 11, 168, 192], [136, 246, 228, 178]),
    // `log(uint,uint,address,string)` -> `log(uint256,uint256,address,string)`
    // `d6a2d1de` -> `6cde40b8`
    ([214, 162, 209, 222], [108, 222, 64, 184]),
    // `log(uint,uint,address,bool)` -> `log(uint256,uint256,address,bool)`
    // `a8e820ae` -> `15cac476`
    ([168, 232, 32, 174], [21, 202, 196, 118]),
    // `log(uint,uint,address,address)` -> `log(uint256,uint256,address,address)`
    // `ca939b20` -> `56a5d1b1`
    ([202, 147, 155, 32], [86, 165, 209, 177]),
    // `log(uint,string,uint,uint)` -> `log(uint256,string,uint256,uint256)`
    // `c0043807` -> `82c25b74`
    ([192, 4, 56, 7], [130, 194, 91, 116]),
    // `log(uint,string,uint,string)` -> `log(uint256,string,uint256,string)`
    // `a2bc0c99` -> `b7b914ca`
    ([162, 188, 12, 153], [183, 185, 20, 202]),
    // `log(uint,string,uint,bool)` -> `log(uint256,string,uint256,bool)`
    // `875a6e2e` -> `691a8f74`
    ([135, 90, 110, 46], [105, 26, 143, 116]),
    // `log(uint,string,uint,address)` -> `log(uint256,string,uint256,address)`
    // `ab7bd9fd` -> `3b2279b4`
    ([171, 123, 217, 253], [59, 34, 121, 180]),
    // `log(uint,string,string,uint)` -> `log(uint256,string,string,uint256)`
    // `76ec635e` -> `b028c9bd`
    ([118, 236, 99, 94], [176, 40, 201, 189]),
    // `log(uint,string,string,string)` -> `log(uint256,string,string,string)`
    // `57dd0a11` -> `21ad0683`
    ([87, 221, 10, 17], [33, 173, 6, 131]),
    // `log(uint,string,string,bool)` -> `log(uint256,string,string,bool)`
    // `12862b98` -> `b3a6b6bd`
    ([18, 134, 43, 152], [179, 166, 182, 189]),
    // `log(uint,string,string,address)` -> `log(uint256,string,string,address)`
    // `cc988aa0` -> `d583c602`
    ([204, 152, 138, 160], [213, 131, 198, 2]),
    // `log(uint,string,bool,uint)` -> `log(uint256,string,bool,uint256)`
    // `a4b48a7f` -> `cf009880`
    ([164, 180, 138, 127], [207, 0, 152, 128]),
    // `log(uint,string,bool,string)` -> `log(uint256,string,bool,string)`
    // `8d489ca0` -> `d2d423cd`
    ([141, 72, 156, 160], [210, 212, 35, 205]),
    // `log(uint,string,bool,bool)` -> `log(uint256,string,bool,bool)`
    // `51bc2bc1` -> `ba535d9c`
    ([81, 188, 43, 193], [186, 83, 93, 156]),
    // `log(uint,string,bool,address)` -> `log(uint256,string,bool,address)`
    // `796f28a0` -> `ae2ec581`
    ([121, 111, 40, 160], [174, 46, 197, 129]),
    // `log(uint,string,address,uint)` -> `log(uint256,string,address,uint256)`
    // `98e7f3f3` -> `e8d3018d`
    ([152, 231, 243, 243], [232, 211, 1, 141]),
    // `log(uint,string,address,string)` -> `log(uint256,string,address,string)`
    // `f898577f` -> `9c3adfa1`
    ([248, 152, 87, 127], [156, 58, 223, 161]),
    // `log(uint,string,address,bool)` -> `log(uint256,string,address,bool)`
    // `f93fff37` -> `90c30a56`
    ([249, 63, 255, 55], [144, 195, 10, 86]),
    // `log(uint,string,address,address)` -> `log(uint256,string,address,address)`
    // `7fa5458b` -> `6168ed61`
    ([127, 165, 69, 139], [97, 104, 237, 97]),
    // `log(uint,bool,uint,uint)` -> `log(uint256,bool,uint256,uint256)`
    // `56828da4` -> `c6acc7a8`
    ([86, 130, 141, 164], [198, 172, 199, 168]),
    // `log(uint,bool,uint,string)` -> `log(uint256,bool,uint256,string)`
    // `e8ddbc56` -> `de03e774`
    ([232, 221, 188, 86], [222, 3, 231, 116]),
    // `log(uint,bool,uint,bool)` -> `log(uint256,bool,uint256,bool)`
    // `d2abc4fd` -> `91a02e2a`
    ([210, 171, 196, 253], [145, 160, 46, 42]),
    // `log(uint,bool,uint,address)` -> `log(uint256,bool,uint256,address)`
    // `4f40058e` -> `88cb6041`
    ([79, 64, 5, 142], [136, 203, 96, 65]),
    // `log(uint,bool,string,uint)` -> `log(uint256,bool,string,uint256)`
    // `915fdb28` -> `2c1d0746`
    ([145, 95, 219, 40], [44, 29, 7, 70]),
    // `log(uint,bool,string,string)` -> `log(uint256,bool,string,string)`
    // `a433fcfd` -> `68c8b8bd`
    ([164, 51, 252, 253], [104, 200, 184, 189]),
    // `log(uint,bool,string,bool)` -> `log(uint256,bool,string,bool)`
    // `346eb8c7` -> `eb928d7f`
    ([52, 110, 184, 199], [235, 146, 141, 127]),
    // `log(uint,bool,string,address)` -> `log(uint256,bool,string,address)`
    // `496e2bb4` -> `ef529018`
    ([73, 110, 43, 180], [239, 82, 144, 24]),
    // `log(uint,bool,bool,uint)` -> `log(uint256,bool,bool,uint256)`
    // `bd25ad59` -> `7464ce23`
    ([189, 37, 173, 89], [116, 100, 206, 35]),
    // `log(uint,bool,bool,string)` -> `log(uint256,bool,bool,string)`
    // `318ae59b` -> `dddb9561`
    ([49, 138, 229, 155], [221, 219, 149, 97]),
    // `log(uint,bool,bool,bool)` -> `log(uint256,bool,bool,bool)`
    // `4e6c5315` -> `b6f577a1`
    ([78, 108, 83, 21], [182, 245, 119, 161]),
    // `log(uint,bool,bool,address)` -> `log(uint256,bool,bool,address)`
    // `5306225d` -> `69640b59`
    ([83, 6, 34, 93], [105, 100, 11, 89]),
    // `log(uint,bool,address,uint)` -> `log(uint256,bool,address,uint256)`
    // `41b5ef3b` -> `078287f5`
    ([65, 181, 239, 59], [7, 130, 135, 245]),
    // `log(uint,bool,address,string)` -> `log(uint256,bool,address,string)`
    // `a230761e` -> `ade052c7`
    ([162, 48, 118, 30], [173, 224, 82, 199]),
    // `log(uint,bool,address,bool)` -> `log(uint256,bool,address,bool)`
    // `91fb1242` -> `454d54a5`
    ([145, 251, 18, 66], [69, 77, 84, 165]),
    // `log(uint,bool,address,address)` -> `log(uint256,bool,address,address)`
    // `86edc10c` -> `a1ef4cbb`
    ([134, 237, 193, 12], [161, 239, 76, 187]),
    // `log(uint,address,uint,uint)` -> `log(uint256,address,uint256,uint256)`
    // `ca9a3eb4` -> `0c9cd9c1`
    ([202, 154, 62, 180], [12, 156, 217, 193]),
    // `log(uint,address,uint,string)` -> `log(uint256,address,uint256,string)`
    // `3ed3bd28` -> `ddb06521`
    ([62, 211, 189, 40], [221, 176, 101, 33]),
    // `log(uint,address,uint,bool)` -> `log(uint256,address,uint256,bool)`
    // `19f67369` -> `5f743a7c`
    ([25, 246, 115, 105], [95, 116, 58, 124]),
    // `log(uint,address,uint,address)` -> `log(uint256,address,uint256,address)`
    // `fdb2ecd4` -> `15c127b5`
    ([253, 178, 236, 212], [21, 193, 39, 181]),
    // `log(uint,address,string,uint)` -> `log(uint256,address,string,uint256)`
    // `a0c414e8` -> `46826b5d`
    ([160, 196, 20, 232], [70, 130, 107, 93]),
    // `log(uint,address,string,string)` -> `log(uint256,address,string,string)`
    // `8d778624` -> `3e128ca3`
    ([141, 119, 134, 36], [62, 18, 140, 163]),
    // `log(uint,address,string,bool)` -> `log(uint256,address,string,bool)`
    // `22a479a6` -> `cc32ab07`
    ([34, 164, 121, 166], [204, 50, 171, 7]),
    // `log(uint,address,string,address)` -> `log(uint256,address,string,address)`
    // `cbe58efd` -> `9cba8fff`
    ([203, 229, 142, 253], [156, 186, 143, 255]),
    // `log(uint,address,bool,uint)` -> `log(uint256,address,bool,uint256)`
    // `7b08e8eb` -> `5abd992a`
    ([123, 8, 232, 235], [90, 189, 153, 42]),
    // `log(uint,address,bool,string)` -> `log(uint256,address,bool,string)`
    // `63f0e242` -> `90fb06aa`
    ([99, 240, 226, 66], [144, 251, 6, 170]),
    // `log(uint,address,bool,bool)` -> `log(uint256,address,bool,bool)`
    // `7e27410d` -> `e351140f`
    ([126, 39, 65, 13], [227, 81, 20, 15]),
    // `log(uint,address,bool,address)` -> `log(uint256,address,bool,address)`
    // `b6313094` -> `ef72c513`
    ([182, 49, 48, 148], [239, 114, 197, 19]),
    // `log(uint,address,address,uint)` -> `log(uint256,address,address,uint256)`
    // `9a3cbf96` -> `736efbb6`
    ([154, 60, 191, 150], [115, 110, 251, 182]),
    // `log(uint,address,address,string)` -> `log(uint256,address,address,string)`
    // `7943dc66` -> `031c6f73`
    ([121, 67, 220, 102], [3, 28, 111, 115]),
    // `log(uint,address,address,bool)` -> `log(uint256,address,address,bool)`
    // `01550b04` -> `091ffaf5`
    ([1, 85, 11, 4], [9, 31, 250, 245]),
    // `log(uint,address,address,address)` -> `log(uint256,address,address,address)`
    // `554745f9` -> `2488b414`
    ([85, 71, 69, 249], [36, 136, 180, 20]),
    // `log(string,uint,uint,uint)` -> `log(string,uint256,uint256,uint256)`
    // `08ee5666` -> `a7a87853`
    ([8, 238, 86, 102], [167, 168, 120, 83]),
    // `log(string,uint,uint,string)` -> `log(string,uint256,uint256,string)`
    // `a54ed4bd` -> `854b3496`
    ([165, 78, 212, 189], [133, 75, 52, 150]),
    // `log(string,uint,uint,bool)` -> `log(string,uint256,uint256,bool)`
    // `f73c7e3d` -> `7626db92`
    ([247, 60, 126, 61], [118, 38, 219, 146]),
    // `log(string,uint,uint,address)` -> `log(string,uint256,uint256,address)`
    // `bed728bf` -> `e21de278`
    ([190, 215, 40, 191], [226, 29, 226, 120]),
    // `log(string,uint,string,uint)` -> `log(string,uint256,string,uint256)`
    // `a0c4b225` -> `c67ea9d1`
    ([160, 196, 178, 37], [198, 126, 169, 209]),
    // `log(string,uint,string,string)` -> `log(string,uint256,string,string)`
    // `6c98dae2` -> `5ab84e1f`
    ([108, 152, 218, 226], [90, 184, 78, 31]),
    // `log(string,uint,string,bool)` -> `log(string,uint256,string,bool)`
    // `e99f82cf` -> `7d24491d`
    ([233, 159, 130, 207], [125, 36, 73, 29]),
    // `log(string,uint,string,address)` -> `log(string,uint256,string,address)`
    // `bb7235e9` -> `7c4632a4`
    ([187, 114, 53, 233], [124, 70, 50, 164]),
    // `log(string,uint,bool,uint)` -> `log(string,uint256,bool,uint256)`
    // `550e6ef5` -> `e41b6f6f`
    ([85, 14, 110, 245], [228, 27, 111, 111]),
    // `log(string,uint,bool,string)` -> `log(string,uint256,bool,string)`
    // `76cc6064` -> `abf73a98`
    ([118, 204, 96, 100], [171, 247, 58, 152]),
    // `log(string,uint,bool,bool)` -> `log(string,uint256,bool,bool)`
    // `e37ff3d0` -> `354c36d6`
    ([227, 127, 243, 208], [53, 76, 54, 214]),
    // `log(string,uint,bool,address)` -> `log(string,uint256,bool,address)`
    // `e5549d91` -> `e0e95b98`
    ([229, 84, 157, 145], [224, 233, 91, 152]),
    // `log(string,uint,address,uint)` -> `log(string,uint256,address,uint256)`
    // `58497afe` -> `4f04fdc6`
    ([88, 73, 122, 254], [79, 4, 253, 198]),
    // `log(string,uint,address,string)` -> `log(string,uint256,address,string)`
    // `3254c2e8` -> `9ffb2f93`
    ([50, 84, 194, 232], [159, 251, 47, 147]),
    // `log(string,uint,address,bool)` -> `log(string,uint256,address,bool)`
    // `1106a8f7` -> `82112a42`
    ([17, 6, 168, 247], [130, 17, 42, 66]),
    // `log(string,uint,address,address)` -> `log(string,uint256,address,address)`
    // `eac89281` -> `5ea2b7ae`
    ([234, 200, 146, 129], [94, 162, 183, 174]),
    // `log(string,string,uint,uint)` -> `log(string,string,uint256,uint256)`
    // `d5cf17d0` -> `f45d7d2c`
    ([213, 207, 23, 208], [244, 93, 125, 44]),
    // `log(string,string,uint,string)` -> `log(string,string,uint256,string)`
    // `8d142cdd` -> `5d1a971a`
    ([141, 20, 44, 221], [93, 26, 151, 26]),
    // `log(string,string,uint,bool)` -> `log(string,string,uint256,bool)`
    // `e65658ca` -> `c3a8a654`
    ([230, 86, 88, 202], [195, 168, 166, 84]),
    // `log(string,string,uint,address)` -> `log(string,string,uint256,address)`
    // `5d4f4680` -> `1023f7b2`
    ([93, 79, 70, 128], [16, 35, 247, 178]),
    // `log(string,string,string,uint)` -> `log(string,string,string,uint256)`
    // `9fd009f5` -> `8eafb02b`
    ([159, 208, 9, 245], [142, 175, 176, 43]),
    // `log(string,string,bool,uint)` -> `log(string,string,bool,uint256)`
    // `86818a7a` -> `d6aefad2`
    ([134, 129, 138, 122], [214, 174, 250, 210]),
    // `log(string,string,address,uint)` -> `log(string,string,address,uint256)`
    // `4a81a56a` -> `7cc3c607`
    ([74, 129, 165, 106], [124, 195, 198, 7]),
    // `log(string,bool,uint,uint)` -> `log(string,bool,uint256,uint256)`
    // `5dbff038` -> `64b5bb67`
    ([93, 191, 240, 56], [100, 181, 187, 103]),
    // `log(string,bool,uint,string)` -> `log(string,bool,uint256,string)`
    // `42b9a227` -> `742d6ee7`
    ([66, 185, 162, 39], [116, 45, 110, 231]),
    // `log(string,bool,uint,bool)` -> `log(string,bool,uint256,bool)`
    // `3cc5b5d3` -> `8af7cf8a`
    ([60, 197, 181, 211], [138, 247, 207, 138]),
    // `log(string,bool,uint,address)` -> `log(string,bool,uint256,address)`
    // `71d3850d` -> `935e09bf`
    ([113, 211, 133, 13], [147, 94, 9, 191]),
    // `log(string,bool,string,uint)` -> `log(string,bool,string,uint256)`
    // `34cb308d` -> `24f91465`
    ([52, 203, 48, 141], [36, 249, 20, 101]),
    // `log(string,bool,bool,uint)` -> `log(string,bool,bool,uint256)`
    // `807531e8` -> `8e3f78a9`
    ([128, 117, 49, 232], [142, 63, 120, 169]),
    // `log(string,bool,address,uint)` -> `log(string,bool,address,uint256)`
    // `28df4e96` -> `5d08bb05`
    ([40, 223, 78, 150], [93, 8, 187, 5]),
    // `log(string,address,uint,uint)` -> `log(string,address,uint256,uint256)`
    // `daa394bd` -> `f8f51b1e`
    ([218, 163, 148, 189], [248, 245, 27, 30]),
    // `log(string,address,uint,string)` -> `log(string,address,uint256,string)`
    // `4c55f234` -> `5a477632`
    ([76, 85, 242, 52], [90, 71, 118, 50]),
    // `log(string,address,uint,bool)` -> `log(string,address,uint256,bool)`
    // `5ac1c13c` -> `fc4845f0`
    ([90, 193, 193, 60], [252, 72, 69, 240]),
    // `log(string,address,uint,address)` -> `log(string,address,uint256,address)`
    // `a366ec80` -> `63fb8bc5`
    ([163, 102, 236, 128], [99, 251, 139, 197]),
    // `log(string,address,string,uint)` -> `log(string,address,string,uint256)`
    // `8f624be9` -> `91d1112e`
    ([143, 98, 75, 233], [145, 209, 17, 46]),
    // `log(string,address,bool,uint)` -> `log(string,address,bool,uint256)`
    // `c5d1bb8b` -> `3e9f866a`
    ([197, 209, 187, 139], [62, 159, 134, 106]),
    // `log(string,address,address,uint)` -> `log(string,address,address,uint256)`
    // `6eb7943d` -> `8ef3f399`
    ([110, 183, 148, 61], [142, 243, 243, 153]),
    // `log(bool,uint,uint,uint)` -> `log(bool,uint256,uint256,uint256)`
    // `32dfa524` -> `374bb4b2`
    ([50, 223, 165, 36], [55, 75, 180, 178]),
    // `log(bool,uint,uint,string)` -> `log(bool,uint256,uint256,string)`
    // `da0666c8` -> `8e69fb5d`
    ([218, 6, 102, 200], [142, 105, 251, 93]),
    // `log(bool,uint,uint,bool)` -> `log(bool,uint256,uint256,bool)`
    // `a41d81de` -> `be984353`
    ([164, 29, 129, 222], [190, 152, 67, 83]),
    // `log(bool,uint,uint,address)` -> `log(bool,uint256,uint256,address)`
    // `f161b221` -> `00dd87b9`
    ([241, 97, 178, 33], [0, 221, 135, 185]),
    // `log(bool,uint,string,uint)` -> `log(bool,uint256,string,uint256)`
    // `4180011b` -> `6a1199e2`
    ([65, 128, 1, 27], [106, 17, 153, 226]),
    // `log(bool,uint,string,string)` -> `log(bool,uint256,string,string)`
    // `d32a6548` -> `f5bc2249`
    ([211, 42, 101, 72], [245, 188, 34, 73]),
    // `log(bool,uint,string,bool)` -> `log(bool,uint256,string,bool)`
    // `91d2f813` -> `e5e70b2b`
    ([145, 210, 248, 19], [229, 231, 11, 43]),
    // `log(bool,uint,string,address)` -> `log(bool,uint256,string,address)`
    // `a5c70d29` -> `fedd1fff`
    ([165, 199, 13, 41], [254, 221, 31, 255]),
    // `log(bool,uint,bool,uint)` -> `log(bool,uint256,bool,uint256)`
    // `d3de5593` -> `7f9bbca2`
    ([211, 222, 85, 147], [127, 155, 188, 162]),
    // `log(bool,uint,bool,string)` -> `log(bool,uint256,bool,string)`
    // `b6d569d4` -> `9143dbb1`
    ([182, 213, 105, 212], [145, 67, 219, 177]),
    // `log(bool,uint,bool,bool)` -> `log(bool,uint256,bool,bool)`
    // `9e01f741` -> `ceb5f4d7`
    ([158, 1, 247, 65], [206, 181, 244, 215]),
    // `log(bool,uint,bool,address)` -> `log(bool,uint256,bool,address)`
    // `4267c7f8` -> `9acd3616`
    ([66, 103, 199, 248], [154, 205, 54, 22]),
    // `log(bool,uint,address,uint)` -> `log(bool,uint256,address,uint256)`
    // `caa5236a` -> `1537dc87`
    ([202, 165, 35, 106], [21, 55, 220, 135]),
    // `log(bool,uint,address,string)` -> `log(bool,uint256,address,string)`
    // `18091341` -> `1bb3b09a`
    ([24, 9, 19, 65], [27, 179, 176, 154]),
    // `log(bool,uint,address,bool)` -> `log(bool,uint256,address,bool)`
    // `65adf408` -> `b4c314ff`
    ([101, 173, 244, 8], [180, 195, 20, 255]),
    // `log(bool,uint,address,address)` -> `log(bool,uint256,address,address)`
    // `8a2f90aa` -> `26f560a8`
    ([138, 47, 144, 170], [38, 245, 96, 168]),
    // `log(bool,string,uint,uint)` -> `log(bool,string,uint256,uint256)`
    // `8e4ae86e` -> `28863fcb`
    ([142, 74, 232, 110], [40, 134, 63, 203]),
    // `log(bool,string,uint,string)` -> `log(bool,string,uint256,string)`
    // `77a1abed` -> `1ad96de6`
    ([119, 161, 171, 237], [26, 217, 109, 230]),
    // `log(bool,string,uint,bool)` -> `log(bool,string,uint256,bool)`
    // `20bbc9af` -> `6b0e5d53`
    ([32, 187, 201, 175], [107, 14, 93, 83]),
    // `log(bool,string,uint,address)` -> `log(bool,string,uint256,address)`
    // `5b22b938` -> `1596a1ce`
    ([91, 34, 185, 56], [21, 150, 161, 206]),
    // `log(bool,string,string,uint)` -> `log(bool,string,string,uint256)`
    // `5ddb2592` -> `7be0c3eb`
    ([93, 219, 37, 146], [123, 224, 195, 235]),
    // `log(bool,string,bool,uint)` -> `log(bool,string,bool,uint256)`
    // `8d6f9ca5` -> `1606a393`
    ([141, 111, 156, 165], [22, 6, 163, 147]),
    // `log(bool,string,address,uint)` -> `log(bool,string,address,uint256)`
    // `1b0b955b` -> `a5cada94`
    ([27, 11, 149, 91], [165, 202, 218, 148]),
    // `log(bool,bool,uint,uint)` -> `log(bool,bool,uint256,uint256)`
    // `4667de8e` -> `0bb00eab`
    ([70, 103, 222, 142], [11, 176, 14, 171]),
    // `log(bool,bool,uint,string)` -> `log(bool,bool,uint256,string)`
    // `50618937` -> `7dd4d0e0`
    ([80, 97, 137, 55], [125, 212, 208, 224]),
    // `log(bool,bool,uint,bool)` -> `log(bool,bool,uint256,bool)`
    // `ab5cc1c4` -> `619e4d0e`
    ([171, 92, 193, 196], [97, 158, 77, 14]),
    // `log(bool,bool,uint,address)` -> `log(bool,bool,uint256,address)`
    // `0bff950d` -> `54a7a9a0`
    ([11, 255, 149, 13], [84, 167, 169, 160]),
    // `log(bool,bool,string,uint)` -> `log(bool,bool,string,uint256)`
    // `178b4685` -> `e3a9ca2f`
    ([23, 139, 70, 133], [227, 169, 202, 47]),
    // `log(bool,bool,bool,uint)` -> `log(bool,bool,bool,uint256)`
    // `c248834d` -> `6d7045c1`
    ([194, 72, 131, 77], [109, 112, 69, 193]),
    // `log(bool,bool,address,uint)` -> `log(bool,bool,address,uint256)`
    // `609386e7` -> `4c123d57`
    ([96, 147, 134, 231], [76, 18, 61, 87]),
    // `log(bool,address,uint,uint)` -> `log(bool,address,uint256,uint256)`
    // `9bfe72bc` -> `7bf181a1`
    ([155, 254, 114, 188], [123, 241, 129, 161]),
    // `log(bool,address,uint,string)` -> `log(bool,address,uint256,string)`
    // `a0685833` -> `51f09ff8`
    ([160, 104, 88, 51], [81, 240, 159, 248]),
    // `log(bool,address,uint,bool)` -> `log(bool,address,uint256,bool)`
    // `ee8d8672` -> `d6019f1c`
    ([238, 141, 134, 114], [214, 1, 159, 28]),
    // `log(bool,address,uint,address)` -> `log(bool,address,uint256,address)`
    // `68f158b5` -> `136b05dd`
    ([104, 241, 88, 181], [19, 107, 5, 221]),
    // `log(bool,address,string,uint)` -> `log(bool,address,string,uint256)`
    // `0b99fc22` -> `c21f64c7`
    ([11, 153, 252, 34], [194, 31, 100, 199]),
    // `log(bool,address,bool,uint)` -> `log(bool,address,bool,uint256)`
    // `4cb60fd1` -> `07831502`
    ([76, 182, 15, 209], [7, 131, 21, 2]),
    // `log(bool,address,address,uint)` -> `log(bool,address,address,uint256)`
    // `5284bd6c` -> `0c66d1be`
    ([82, 132, 189, 108], [12, 102, 209, 190]),
    // `log(address,uint,uint,uint)` -> `log(address,uint256,uint256,uint256)`
    // `3d0e9de4` -> `34f0e636`
    ([61, 14, 157, 228], [52, 240, 230, 54]),
    // `log(address,uint,uint,string)` -> `log(address,uint256,uint256,string)`
    // `89340dab` -> `4a28c017`
    ([137, 52, 13, 171], [74, 40, 192, 23]),
    // `log(address,uint,uint,bool)` -> `log(address,uint256,uint256,bool)`
    // `ec4ba8a2` -> `66f1bc67`
    ([236, 75, 168, 162], [102, 241, 188, 103]),
    // `log(address,uint,uint,address)` -> `log(address,uint256,uint256,address)`
    // `1ef63434` -> `20e3984d`
    ([30, 246, 52, 52], [32, 227, 152, 77]),
    // `log(address,uint,string,uint)` -> `log(address,uint256,string,uint256)`
    // `f512cf9b` -> `bf01f891`
    ([245, 18, 207, 155], [191, 1, 248, 145]),
    // `log(address,uint,string,string)` -> `log(address,uint256,string,string)`
    // `7e56c693` -> `88a8c406`
    ([126, 86, 198, 147], [136, 168, 196, 6]),
    // `log(address,uint,string,bool)` -> `log(address,uint256,string,bool)`
    // `a4024f11` -> `cf18105c`
    ([164, 2, 79, 17], [207, 24, 16, 92]),
    // `log(address,uint,string,address)` -> `log(address,uint256,string,address)`
    // `dc792604` -> `5c430d47`
    ([220, 121, 38, 4], [92, 67, 13, 71]),
    // `log(address,uint,bool,uint)` -> `log(address,uint256,bool,uint256)`
    // `698f4392` -> `22f6b999`
    ([105, 143, 67, 146], [34, 246, 185, 153]),
    // `log(address,uint,bool,string)` -> `log(address,uint256,bool,string)`
    // `8e8e4e75` -> `c5ad85f9`
    ([142, 142, 78, 117], [197, 173, 133, 249]),
    // `log(address,uint,bool,bool)` -> `log(address,uint256,bool,bool)`
    // `fea1d55a` -> `3bf5e537`
    ([254, 161, 213, 90], [59, 245, 229, 55]),
    // `log(address,uint,bool,address)` -> `log(address,uint256,bool,address)`
    // `23e54972` -> `a31bfdcc`
    ([35, 229, 73, 114], [163, 27, 253, 204]),
    // `log(address,uint,address,uint)` -> `log(address,uint256,address,uint256)`
    // `a5d98768` -> `100f650e`
    ([165, 217, 135, 104], [16, 15, 101, 14]),
    // `log(address,uint,address,string)` -> `log(address,uint256,address,string)`
    // `5d71f39e` -> `1da986ea`
    ([93, 113, 243, 158], [29, 169, 134, 234]),
    // `log(address,uint,address,bool)` -> `log(address,uint256,address,bool)`
    // `f181a1e9` -> `a1bcc9b3`
    ([241, 129, 161, 233], [161, 188, 201, 179]),
    // `log(address,uint,address,address)` -> `log(address,uint256,address,address)`
    // `ec24846f` -> `478d1c62`
    ([236, 36, 132, 111], [71, 141, 28, 98]),
    // `log(address,string,uint,uint)` -> `log(address,string,uint256,uint256)`
    // `a4c92a60` -> `1dc8e1b8`
    ([164, 201, 42, 96], [29, 200, 225, 184]),
    // `log(address,string,uint,string)` -> `log(address,string,uint256,string)`
    // `5d1365c9` -> `448830a8`
    ([93, 19, 101, 201], [68, 136, 48, 168]),
    // `log(address,string,uint,bool)` -> `log(address,string,uint256,bool)`
    // `7e250d5b` -> `0ef7e050`
    ([126, 37, 13, 91], [14, 247, 224, 80]),
    // `log(address,string,uint,address)` -> `log(address,string,uint256,address)`
    // `dfd7d80b` -> `63183678`
    ([223, 215, 216, 11], [99, 24, 54, 120]),
    // `log(address,string,string,uint)` -> `log(address,string,string,uint256)`
    // `a14fd039` -> `159f8927`
    ([161, 79, 208, 57], [21, 159, 137, 39]),
    // `log(address,string,bool,uint)` -> `log(address,string,bool,uint256)`
    // `e720521c` -> `515e38b6`
    ([231, 32, 82, 28], [81, 94, 56, 182]),
    // `log(address,string,address,uint)` -> `log(address,string,address,uint256)`
    // `8c1933a9` -> `457fe3cf`
    ([140, 25, 51, 169], [69, 127, 227, 207]),
    // `log(address,bool,uint,uint)` -> `log(address,bool,uint256,uint256)`
    // `c210a01e` -> `386ff5f4`
    ([194, 16, 160, 30], [56, 111, 245, 244]),
    // `log(address,bool,uint,string)` -> `log(address,bool,uint256,string)`
    // `9b588ecc` -> `0aa6cfad`
    ([155, 88, 142, 204], [10, 166, 207, 173]),
    // `log(address,bool,uint,bool)` -> `log(address,bool,uint256,bool)`
    // `85cdc5af` -> `c4643e20`
    ([133, 205, 197, 175], [196, 100, 62, 32]),
    // `log(address,bool,uint,address)` -> `log(address,bool,uint256,address)`
    // `0d8ce61e` -> `ccf790a1`
    ([13, 140, 230, 30], [204, 247, 144, 161]),
    // `log(address,bool,string,uint)` -> `log(address,bool,string,uint256)`
    // `9e127b6e` -> `80e6a20b`
    ([158, 18, 123, 110], [128, 230, 162, 11]),
    // `log(address,bool,bool,uint)` -> `log(address,bool,bool,uint256)`
    // `cfb58756` -> `8c4e5de6`
    ([207, 181, 135, 86], [140, 78, 93, 230]),
    // `log(address,bool,address,uint)` -> `log(address,bool,address,uint256)`
    // `dc7116d2` -> `a75c59de`
    ([220, 113, 22, 210], [167, 92, 89, 222]),
    // `log(address,address,uint,uint)` -> `log(address,address,uint256,uint256)`
    // `54fdf3e4` -> `be553481`
    ([84, 253, 243, 228], [190, 85, 52, 129]),
    // `log(address,address,uint,string)` -> `log(address,address,uint256,string)`
    // `9dd12ead` -> `fdb4f990`
    ([157, 209, 46, 173], [253, 180, 249, 144]),
    // `log(address,address,uint,bool)` -> `log(address,address,uint256,bool)`
    // `c2f688ec` -> `9b4254e2`
    ([194, 246, 136, 236], [155, 66, 84, 226]),
    // `log(address,address,uint,address)` -> `log(address,address,uint256,address)`
    // `d6c65276` -> `8da6def5`
    ([214, 198, 82, 118], [141, 166, 222, 245]),
    // `log(address,address,string,uint)` -> `log(address,address,string,uint256)`
    // `04289300` -> `ef1cefe7`
    ([4, 40, 147, 0], [239, 28, 239, 231]),
    // `log(address,address,bool,uint)` -> `log(address,address,bool,uint256)`
    // `95d65f11` -> `3971e78c`
    ([149, 214, 95, 17], [57, 113, 231, 140]),
    // `log(address,address,address,uint)` -> `log(address,address,address,uint256)`
    // `ed5eac87` -> `94250d77`
    ([237, 94, 172, 135], [148, 37, 13, 119]),
]
