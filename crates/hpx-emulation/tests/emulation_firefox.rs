#[macro_use]
mod support;

use hpx_emulation::Emulation;

test_emulation!(
    test_firefox_109,
    Emulation::Firefox109,
    ["t13d1713h2_5b57614c22b0_4b99b185ee6b"],
    "3d9132023bf26a71d40fe766e5c24c9d"
);

test_emulation!(
    test_firefox_117,
    Emulation::Firefox117,
    ["t13d1713h2_5b57614c22b0_4b99b185ee6b"],
    "3d9132023bf26a71d40fe766e5c24c9d"
);

test_emulation!(
    test_firefox_128,
    Emulation::Firefox128,
    [
        "t13d1512h2_8daaf6152771_aa8ec201ac7d",
        "t13d1513h2_8daaf6152771_aa8ec201ac7d"
    ],
    "1d8a6f51fd7253d04674593073fc18b0"
);

test_emulation!(
    test_firefox_133,
    Emulation::Firefox133,
    ["t13d1714h2_5b57614c22b0_6feedaa227c1"],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_135,
    Emulation::Firefox135,
    [
        "t13d1715h2_5b57614c22b0_1560211d49fe",
        "t13d1715h2_5b57614c22b0_c1eccb039341"
    ],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_private_135,
    Emulation::FirefoxPrivate135,
    ["t13d1714h2_5b57614c22b0_e66382aaeb1d"],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_android_135,
    Emulation::FirefoxAndroid135,
    [
        "t13d1714h2_5b57614c22b0_6feedaa227c1",
        "t13d1715h2_5b57614c22b0_3fb10893fbe6",
        "t13d1715h2_5b57614c22b0_6feedaa227c1"
    ],
    "41a06cadb1c6385e6d08c8d0dbbea818"
);

test_emulation!(
    test_firefox_136,
    Emulation::Firefox136,
    [
        "t13d1715h2_5b57614c22b0_1560211d49fe",
        "t13d1715h2_5b57614c22b0_c1eccb039341"
    ],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_private_136,
    Emulation::FirefoxPrivate136,
    ["t13d1714h2_5b57614c22b0_e66382aaeb1d"],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_139,
    Emulation::Firefox139,
    [
        "t13d1715h2_5b57614c22b0_c1eccb039341",
        "t13d1715h2_5b57614c22b0_1560211d49fe"
    ],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_142,
    Emulation::Firefox142,
    [
        "t13d1715h2_5b57614c22b0_1560211d49fe",
        "t13d1715h2_5b57614c22b0_c1eccb039341"
    ],
    "6ea73faa8fc5aac76bded7bd238f6433"
);

test_emulation!(
    test_firefox_143,
    Emulation::Firefox143,
    [
        "t13d1715h2_5b57614c22b0_1560211d49fe",
        "t13d1715h2_5b57614c22b0_c1eccb039341"
    ],
    "6ea73faa8fc5aac76bded7bd238f6433"
);
