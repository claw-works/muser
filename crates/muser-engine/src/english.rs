//! 英文歌词 → 音素（ARPAbet）→ 音节，给 `yinse.voice.*` 唱（[`crate::singer`]）。
//!
//! 不带发音词典：一份常用的不规则词表（几百个，歌词里最常见的那些）加拼写规则。规则读错的词，歌词里直接写音素
//! （`S-T-R-IY-T`，全大写、用 `-` 连起来）。一个音符唱一个音节；一个词有几个音节，后面的音符写 `+` 接着唱下一个。

/// 元音音素。
const VOWELS: [&str; 16] = ["AA", "AE", "AH", "AO", "AW", "AX", "AY", "EH", "ER", "EY", "IH", "IY", "OW", "OY", "UH", "UW"];

/// 不规则的常用词（拼写规则读不对的）。
const WORDS: &[(&str, &str)] = &[
    ("a", "AX"), ("the", "DH AX"), ("of", "AX V"), ("to", "T UW"), ("do", "D UW"), ("who", "HH UW"), ("you", "Y UW"), ("your", "Y AO R"), ("yours", "Y AO R Z"),
    ("i", "AY"), ("im", "AY M"), ("ill", "AY L"), ("ive", "AY V"), ("id", "AY D"), ("me", "M IY"), ("my", "M AY"), ("we", "W IY"), ("he", "HH IY"), ("she", "SH IY"), ("be", "B IY"),
    ("are", "AA R"), ("is", "IH Z"), ("was", "W AH Z"), ("were", "W ER"), ("am", "AE M"), ("has", "HH AE Z"), ("have", "HH AE V"), ("had", "HH AE D"), ("his", "HH IH Z"), ("as", "AE Z"),
    ("this", "DH IH S"), ("that", "DH AE T"), ("these", "DH IY Z"), ("those", "DH OW Z"), ("there", "DH EH R"), ("their", "DH EH R"), ("they", "DH EY"), ("them", "DH EH M"), ("then", "DH EH N"), ("than", "DH AE N"), ("though", "DH OW"), ("thus", "DH AH S"), ("with", "W IH DH"), ("without", "W IH DH AW T"),
    ("one", "W AH N"), ("once", "W AH N S"), ("two", "T UW"), ("four", "F AO R"), ("eight", "EY T"), ("done", "D AH N"), ("none", "N AH N"), ("gone", "G AO N"), ("some", "S AH M"), ("come", "K AH M"), ("love", "L AH V"), ("above", "AX B AH V"), ("dove", "D AH V"), ("glove", "G L AH V"),
    ("give", "G IH V"), ("live", "L IH V"), ("forgive", "F ER G IH V"), ("get", "G EH T"), ("forget", "F ER G EH T"), ("girl", "G ER L"), ("begin", "B IH G IH N"), ("again", "AX G EH N"), ("against", "AX G EH N S T"), ("together", "T AX G EH DH ER"), ("tiger", "T AY G ER"), ("anger", "AE NG G ER"), ("finger", "F IH NG G ER"), ("singer", "S IH NG ER"),
    ("said", "S EH D"), ("says", "S EH Z"), ("does", "D AH Z"), ("dont", "D OW N T"), ("wont", "W OW N T"), ("cant", "K AE N T"), ("could", "K UH D"), ("would", "W UH D"), ("should", "SH UH D"), ("been", "B IH N"),
    ("what", "W AH T"), ("want", "W AA N T"), ("water", "W AO T ER"), ("walk", "W AO K"), ("talk", "T AO K"), ("watch", "W AA CH"), ("wash", "W AA SH"), ("where", "W EH R"), ("when", "W EH N"), ("why", "W AY"), ("which", "W IH CH"), ("while", "W AY L"), ("white", "W AY T"), ("whole", "HH OW L"), ("whose", "HH UW Z"),
    ("how", "HH AW"), ("now", "N AW"), ("cow", "K AW"), ("wow", "W AW"), ("bow", "B AW"), ("allow", "AX L AW"), ("own", "OW N"), ("known", "N OW N"), ("shown", "SH OW N"), ("grown", "G R OW N"), ("blown", "B L OW N"), ("thrown", "TH R OW N"), ("lower", "L OW ER"),
    ("people", "P IY P AX L"), ("little", "L IH T AX L"), ("beautiful", "B Y UW T IH F AX L"), ("because", "B IH K AO Z"), ("through", "TH R UW"), ("enough", "IH N AH F"), ("tough", "T AH F"), ("rough", "R AH F"), ("laugh", "L AE F"), ("thought", "TH AO T"), ("brought", "B R AO T"), ("caught", "K AO T"), ("taught", "T AO T"), ("daughter", "D AO T ER"),
    ("heart", "HH AA R T"), ("hear", "HH IY R"), ("here", "HH IY R"), ("heard", "HH ER D"), ("earth", "ER TH"), ("early", "ER L IY"), ("learn", "L ER N"), ("bear", "B EH R"), ("wear", "W EH R"), ("tear", "T IY R"), ("year", "Y IY R"),
    ("great", "G R EY T"), ("break", "B R EY K"), ("head", "HH EH D"), ("dead", "D EH D"), ("bread", "B R EH D"), ("instead", "IH N S T EH D"), ("read", "R IY D"), ("ready", "R EH D IY"), ("heaven", "HH EH V AX N"), ("heavy", "HH EH V IY"), ("weather", "W EH DH ER"), ("feather", "F EH DH ER"), ("breath", "B R EH TH"), ("breathe", "B R IY DH"), ("death", "D EH TH"),
    ("friend", "F R EH N D"), ("eye", "AY"), ("eyes", "AY Z"), ("bye", "B AY"), ("goodbye", "G UH D B AY"), ("buy", "B AY"), ("guy", "G AY"), ("high", "HH AY"), ("sigh", "S AY"),
    ("good", "G UH D"), ("wood", "W UH D"), ("stood", "S T UH D"), ("foot", "F UH T"), ("book", "B UH K"), ("look", "L UH K"), ("took", "T UH K"), ("food", "F UW D"), ("mood", "M UW D"), ("blood", "B L AH D"), ("flood", "F L AH D"), ("door", "D AO R"), ("floor", "F L AO R"), ("poor", "P UH R"),
    ("young", "Y AH NG"), ("touch", "T AH CH"), ("country", "K AH N T R IY"), ("double", "D AH B AX L"), ("trouble", "T R AH B AX L"), ("group", "G R UW P"), ("soul", "S OW L"), ("shoulder", "SH OW L D ER"), ("soup", "S UW P"), ("course", "K AO R S"), ("pour", "P AO R"), ("hour", "AW ER"), ("our", "AW ER"),
    ("mother", "M AH DH ER"), ("father", "F AA DH ER"), ("brother", "B R AH DH ER"), ("other", "AH DH ER"), ("another", "AX N AH DH ER"), ("nothing", "N AH TH IH NG"), ("something", "S AH M TH IH NG"), ("everything", "EH V R IY TH IH NG"), ("anything", "EH N IY TH IH NG"),
    ("every", "EH V R IY"), ("everyone", "EH V R IY W AH N"), ("never", "N EH V ER"), ("ever", "EH V ER"), ("forever", "F ER EH V ER"), ("over", "OW V ER"), ("under", "AH N D ER"), ("after", "AE F T ER"), ("only", "OW N L IY"), ("any", "EH N IY"), ("many", "M EH N IY"), ("very", "V EH R IY"), ("into", "IH N T UW"), ("onto", "AA N T UW"),
    ("even", "IY V AX N"), ("evening", "IY V N IH NG"), ("open", "OW P AX N"), ("paper", "P EY P ER"), ("later", "L EY T ER"), ("lady", "L EY D IY"), ("baby", "B EY B IY"), ("maybe", "M EY B IY"), ("body", "B AA D IY"), ("city", "S IH T IY"), ("pretty", "P R IH T IY"), ("busy", "B IH Z IY"), ("study", "S T AH D IY"), ("money", "M AH N IY"), ("honey", "HH AH N IY"),
    ("woman", "W UH M AX N"), ("women", "W IH M IH N"), ("human", "HH Y UW M AX N"), ("music", "M Y UW Z IH K"), ("moment", "M OW M AX N T"), ("silent", "S AY L AX N T"), ("secret", "S IY K R IH T"), ("final", "F AY N AX L"), ("spider", "S P AY D ER"), ("river", "R IH V ER"), ("seven", "S EH V AX N"), ("lover", "L AH V ER"), ("cover", "K AH V ER"), ("color", "K AH L ER"), ("wonder", "W AH N D ER"), ("wonderful", "W AH N D ER F AX L"), ("summer", "S AH M ER"), ("winter", "W IH N T ER"),
    ("world", "W ER L D"), ("word", "W ER D"), ("work", "W ER K"), ("worth", "W ER TH"), ("war", "W AO R"), ("warm", "W AO R M"), ("sure", "SH UH R"), ("sugar", "SH UH G ER"), ("ocean", "OW SH AX N"), ("island", "AY L AX N D"), ("listen", "L IH S AX N"), ("often", "AO F AX N"), ("castle", "K AE S AX L"), ("whistle", "W IH S AX L"), ("answer", "AE N S ER"), ("sword", "S AO R D"),
    ("tonight", "T AX N AY T"), ("today", "T AX D EY"), ("tomorrow", "T AX M AA R OW"), ("always", "AO L W EY Z"), ("away", "AX W EY"), ("alone", "AX L OW N"), ("along", "AX L AO NG"), ("around", "AX R AW N D"), ("about", "AX B AW T"), ("ago", "AX G OW"), ("alive", "AX L AY V"), ("asleep", "AX S L IY P"), ("across", "AX K R AO S"), ("before", "B IH F AO R"), ("believe", "B IH L IY V"), ("behind", "B IH HH AY N D"), ("between", "B IH T W IY N"), ("become", "B IH K AH M"), ("remember", "R IH M EH M B ER"),
    ("oh", "OW"), ("ooh", "UW"), ("ah", "AA"), ("la", "L AA"), ("na", "N AA"), ("yeah", "Y EH"), ("hey", "HH EY"), ("hi", "HH AY"), ("hello", "HH AX L OW"), ("okay", "OW K EY"), ("yes", "Y EH S"), ("no", "N OW"), ("so", "S OW"), ("go", "G OW"), ("us", "AH S"), ("bus", "B AH S"), ("its", "IH T S"), ("lets", "L EH T S"), ("thats", "DH AE T S"), ("whats", "W AH T S"),
    ("child", "CH AY L D"), ("children", "CH IH L D R AX N"), ("wind", "W IH N D"), ("lost", "L AO S T"), ("cost", "K AO S T"), ("most", "M OW S T"), ("both", "B OW TH"), ("move", "M UW V"), ("prove", "P R UW V"), ("lose", "L UW Z"), ("shoe", "SH UW"), ("true", "T R UW"), ("blue", "B L UW"), ("new", "N UW"), ("few", "F Y UW"), ("view", "V Y UW"), ("use", "Y UW Z"), ("used", "Y UW Z D"), ("house", "HH AW S"), ("mouse", "M AW S"), ("else", "EH L S"), ("horse", "HH AO R S"),
    ("diamond", "D AY M AX N D"), ("imagine", "IH M AE JH IH N"), ("religion", "R IH L IH JH AX N"), ("someday", "S AH M D EY"), ("sometimes", "S AH M T AY M Z"), ("somewhere", "S AH M W EH R"), ("somebody", "S AH M B AA D IY"), ("someone", "S AH M W AH N"), ("mountain", "M AW N T AX N"), ("fountain", "F AW N T AX N"), ("valley", "V AE L IY"), ("alright", "AO L R AY T"), ("below", "B IH L OW"), ("minute", "M IH N IH T"), ("promise", "P R AA M IH S"), ("surprise", "S ER P R AY Z"), ("paradise", "P EH R AX D AY S"), ("memories", "M EH M ER IY Z"), ("america", "AX M EH R IH K AX"), ("radio", "R EY D IY OW"), ("piano", "P IY AE N OW"), ("guitar", "G IH T AA R"), ("hallelujah", "HH AE L AX L UW Y AX"),
    ("amazing", "AX M EY Z IH NG"), ("grace", "G R EY S"), ("found", "F AW N D"), ("round", "R AW N D"), ("sound", "S AW N D"),
    ("angel", "EY N JH AX L"), ("danger", "D EY N JH ER"), ("stranger", "S T R EY N JH ER"), ("change", "CH EY N JH"), ("strange", "S T R EY N JH"), ("orange", "AO R IH N JH"), ("magic", "M AE JH IH K"), ("image", "IH M IH JH"), ("language", "L AE NG G W IH JH"), ("school", "S K UW L"), ("christmas", "K R IH S M AX S"), ("stomach", "S T AH M AX K"), ("machine", "M AX SH IY N"),
    ("sign", "S AY N"), ("design", "D IH Z AY N"), ("climb", "K L AY M"), ("lamb", "L AE M"), ("dumb", "D AH M"), ("thumb", "TH AH M"), ("half", "HH AE F"), ("calm", "K AA M"), ("palm", "P AA M"), ("knife", "N AY F"), ("knee", "N IY"), ("know", "N OW"), ("knew", "N UW"), ("write", "R AY T"), ("wrong", "R AO NG"), ("hour", "AW ER"), ("honest", "AA N IH S T"),
    ("fire", "F AY ER"), ("desire", "D IH Z AY ER"), ("higher", "HH AY ER"), ("flower", "F L AW ER"), ("power", "P AW ER"), ("tower", "T AW ER"), ("shower", "SH AW ER"), ("pure", "P Y UH R"), ("cure", "K Y UH R"), ("future", "F Y UW CH ER"), ("picture", "P IH K CH ER"), ("nature", "N EY CH ER"), ("pleasure", "P L EH ZH ER"), ("treasure", "T R EH ZH ER"), ("measure", "M EH ZH ER"),
    ("lazy", "L EY Z IY"), ("crazy", "K R EY Z IY"), ("tiny", "T AY N IY"), ("story", "S T AO R IY"), ("glory", "G L AO R IY"), ("sorry", "S AA R IY"), ("happy", "HH AE P IY"), ("family", "F AE M L IY"), ("memory", "M EH M ER IY"), ("melody", "M EH L AX D IY"), ("coffee", "K AO F IY"), ("thousand", "TH AW Z AX N D"), ("thirty", "TH ER T IY"), ("number", "N AH M B ER"), ("numbers", "N AH M B ER Z"), ("single", "S IH NG G AX L"), ("robot", "R OW B AA T"), ("dance", "D AE N S"), ("chance", "CH AE N S"), ("voice", "V OY S"), ("noise", "N OY Z"), ("choice", "CH OY S"),
];

fn is_vowel(p: &str) -> bool {
    VOWELS.contains(&p)
}

fn vowel_letter(c: char) -> bool {
    "aeiouy".contains(c)
}

/// 一个英文词的音素。先查不规则词表，再试去掉常见的词尾（-s -ed -ing -ly），最后按拼写规则读。
pub fn phonemes(word: &str) -> Vec<&'static str> {
    let w: String = word.to_lowercase().chars().filter(|c| c.is_ascii_lowercase()).collect();
    if w.is_empty() {
        return Vec::new();
    }
    let known = |w: &str| WORDS.iter().find(|(k, _)| *k == w).map(|(_, p)| p.split(' ').collect::<Vec<&'static str>>());
    if let Some(p) = known(&w) {
        return p;
    }
    let base = |stem: &str| -> Vec<&'static str> {
        // 词干：原样、补上 e（lov-ed → love）、去掉双写的辅音（runn-ing → run）。
        let bytes = stem.as_bytes();
        let doubled = bytes.len() >= 3 && bytes[bytes.len() - 1] == bytes[bytes.len() - 2] && !vowel_letter(bytes[bytes.len() - 1] as char) && !stem.ends_with("ll") && !stem.ends_with("ss");
        let with_e = format!("{stem}e");
        if let Some(p) = known(stem).or_else(|| known(&with_e)) {
            return p;
        }
        if doubled {
            return known(&stem[..stem.len() - 1]).unwrap_or_else(|| rules(&stem[..stem.len() - 1]));
        }
        // 一个元音 + 一个辅音结尾的词干（mak-、hop-、rid-）原来有个不发音的 e。
        let groups = stem.chars().fold((0, false), |(n, was), c| if vowel_letter(c) { (n + usize::from(!was), true) } else { (n, false) }).0;
        let last = stem.chars().last().unwrap_or(' ');
        let before = stem.chars().rev().nth(1).unwrap_or(' ');
        let twice = stem.chars().rev().nth(2).is_some_and(vowel_letter);
        if groups == 1 && !vowel_letter(last) && !"wxy".contains(last) && vowel_letter(before) && !twice {
            return rules(&with_e);
        }
        rules(stem)
    };
    let voiceless = |p: &str| ["P", "T", "K", "F", "TH", "S", "SH", "CH"].contains(&p);
    let sibilant = |p: &str| ["S", "Z", "SH", "ZH", "CH", "JH"].contains(&p);
    let stemmed = |cut: usize| w[..w.len() - cut].chars().any(vowel_letter);
    if w.len() > 4 && w.ends_with("ing") && stemmed(3) {
        let mut p = base(&w[..w.len() - 3]);
        p.extend(["IH", "NG"]);
        return p;
    }
    if w.len() > 3 && w.ends_with("ed") && !w.ends_with("eed") && stemmed(2) {
        let mut p = base(&w[..w.len() - 2]);
        match p.last().copied() {
            Some("T" | "D") => p.extend(["IH", "D"]),
            Some(l) if voiceless(l) => p.push("T"),
            _ => p.push("D"),
        }
        return p;
    }
    if w.len() > 3 && w.ends_with("ly") && stemmed(2) {
        let mut p = base(&w[..w.len() - 2]);
        p.extend(["L", "IY"]);
        return p;
    }
    if w.len() > 3 && w.ends_with('s') && !["ss", "us", "is", "as", "os"].iter().any(|e| w.ends_with(e)) {
        let stem = if w.ends_with("ies") { format!("{}y", &w[..w.len() - 3]) } else if w.ends_with("es") && ["sh", "ch", "ss", "x", "z"].iter().any(|e| w[..w.len() - 2].ends_with(e)) { w[..w.len() - 2].to_string() } else { w[..w.len() - 1].to_string() };
        let mut p = known(&stem).unwrap_or_else(|| rules(&stem));
        match p.last().copied() {
            Some(l) if sibilant(l) => p.extend(["IH", "Z"]),
            Some(l) if voiceless(l) => p.push("S"),
            _ => p.push("Z"),
        }
        return p;
    }
    rules(&w)
}

/// 拼写规则：从左到右，每一处先试长的拼法（tion、igh、tch …），再试两个字母的，最后是单个字母。
fn rules(w: &str) -> Vec<&'static str> {
    let b: Vec<char> = w.chars().collect();
    let n = b.len();
    let at = |i: usize| b.get(i).copied().unwrap_or(' ');
    let starts = |i: usize, pat: &str| pat.chars().enumerate().all(|(k, c)| at(i + k) == c);
    let ends = |i: usize, pat: &str| i + pat.len() == n && starts(i, pat);
    let groups = b.iter().fold((0, false), |(c, was), &ch| if vowel_letter(ch) { (c + usize::from(!was), true) } else { (c, false) }).0;
    let many = groups >= 2;
    let mut out: Vec<&'static str> = Vec::new();
    let mut i = 0;
    macro_rules! say {
        ($len:expr, $($p:expr),*) => {{ $( out.push($p); )* i += $len; continue; }};
    }
    while i < n {
        let c = b[i];
        let prev = if i > 0 { b[i - 1] } else { ' ' };
        let next = at(i + 1);
        let prev_v = vowel_letter(prev);
        // —— 几个字母合起来的 ——
        if starts(i, "tion") || starts(i, "ssion") || starts(i, "cian") { say!(if c == 's' { 5 } else { 4 }, "SH", "AX", "N"); }
        if starts(i, "sion") { if prev_v { say!(4, "ZH", "AX", "N"); } else { say!(4, "SH", "AX", "N"); } }
        if ends(i, "ture") { say!(4, "CH", "ER"); }
        if ends(i, "sure") && i > 0 { say!(4, "ZH", "ER"); }
        if starts(i, "cious") || starts(i, "tious") { say!(5, "SH", "AX", "S"); }
        if starts(i, "cial") || starts(i, "tial") { say!(4, "SH", "AX", "L"); }
        if ends(i, "ous") && i > 0 { say!(3, "AX", "S"); }
        if starts(i, "ought") || starts(i, "aught") { say!(5, "AO", "T"); }
        if starts(i, "ough") { say!(4, "OW"); }
        if starts(i, "augh") { say!(4, "AO"); }
        if starts(i, "eigh") { say!(4, "EY"); }
        if starts(i, "igh") { say!(3, "AY"); }
        if starts(i, "tch") { say!(3, "CH"); }
        if starts(i, "dge") { say!(3, "JH"); }
        if starts(i, "ould") { say!(4, "UH", "D"); }
        if ends(i, "ment") && i > 0 { say!(4, "M", "AX", "N", "T"); }
        if ends(i, "ness") && i > 0 { say!(4, "N", "AX", "S"); }
        if ends(i, "less") && i > 0 { say!(4, "L", "AX", "S"); }
        if ends(i, "ful") && i > 0 { say!(3, "F", "AX", "L"); }
        if ends(i, "able") && i >= 3 { say!(4, "AX", "B", "AX", "L"); }
        if ends(i, "le") && i > 0 && !prev_v { say!(2, "AX", "L"); }
        // —— 辅音 ——
        match c {
            'c' => {
                if next == 'h' { if at(i + 2) == 'r' { say!(2, "K"); } else { say!(2, "CH"); } }
                if next == 'k' { say!(2, "K"); }
                if next == 'c' && "eiy".contains(at(i + 2)) { say!(2, "K", "S"); }
                if next == 'c' { say!(2, "K"); }
                if "eiy".contains(next) { say!(1, "S"); }
                say!(1, "K");
            }
            's' => {
                if next == 'h' { say!(2, "SH"); }
                if next == 's' { say!(2, "S"); }
                if starts(i, "sc") && "eiy".contains(at(i + 2)) { say!(2, "S"); }
                // 两个元音之间、元音后面词尾的 se：z（rose、music、please）；ouse 是 s。
                if prev_v && vowel_letter(next) && !(i >= 2 && starts(i - 2, "ouse")) { say!(1, "Z"); }
                say!(1, "S");
            }
            't' => {
                if next == 'h' {
                    // 元音之间后面跟 er 的 th 带声（mother、weather）；其余不带声。带声的常用词在词表里。
                    if prev_v && starts(i + 2, "er") { say!(2, "DH"); }
                    say!(2, "TH");
                }
                if next == 't' { say!(2, "T"); }
                say!(1, "T");
            }
            'p' => {
                if next == 'h' {
                    say!(2, "F");
                }
                say!(if next == 'p' { 2 } else { 1 }, "P");
            }
            'w' => {
                if next == 'h' { say!(2, "W"); }
                if next == 'r' && i == 0 { say!(2, "R"); }
                if vowel_letter(next) { say!(1, "W"); }
                i += 1;
                continue;
            }
            'q' => { if next == 'u' { say!(2, "K", "W"); } say!(1, "K"); }
            'k' => {
                if next == 'n' && i == 0 {
                    say!(2, "N");
                }
                say!(if next == 'k' { 2 } else { 1 }, "K");
            }
            'g' => {
                if next == 'n' && (i == 0 || i + 2 == n) { say!(2, "N"); }
                if next == 'h' { if i == 0 { say!(2, "G"); } else { i += 2; continue; } }
                if next == 'g' { say!(2, "G"); }
                if next == 'u' && "ei".contains(at(i + 2)) { say!(2, "G"); }
                if "eiy".contains(next) { say!(1, "JH"); }
                say!(1, "G");
            }
            'n' => {
                if next == 'g' {
                    if at(i + 2) == 'e' { say!(2, "N", "JH"); }
                    if vowel_letter(at(i + 2)) { say!(2, "NG", "G"); }
                    say!(2, "NG");
                }
                if next == 'k' { say!(1, "NG"); }
                if next == 'n' { say!(2, "N"); }
                say!(1, "N");
            }
            'm' => {
                let silent = (next == 'b' || next == 'n') && i + 2 == n;
                say!(if silent || next == 'm' { 2 } else { 1 }, "M");
            }
            'x' => { if i == 0 { say!(1, "Z"); } say!(1, "K", "S"); }
            'h' => {
                // 元音后面、不在元音前面的 h 不发音（oh、ah）。
                if prev_v && !vowel_letter(next) { i += 1; continue; }
                say!(1, "HH");
            }
            'j' => say!(1, "JH"),
            'v' => say!(1, "V"),
            'z' => say!(if next == 'z' { 2 } else { 1 }, "Z"),
            'f' => say!(if next == 'f' { 2 } else { 1 }, "F"),
            'l' => say!(if next == 'l' { 2 } else { 1 }, "L"),
            'r' => say!(if next == 'r' { 2 } else { 1 }, "R"),
            'd' => say!(if next == 'd' { 2 } else { 1 }, "D"),
            'b' => say!(if next == 'b' { 2 } else { 1 }, "B"),
            'y' if i == 0 && vowel_letter(next) => say!(1, "Y"),
            _ => {}
        }
        // —— 元音 ——
        // 带 r 的。
        if starts(i, "eer") || (starts(i, "ear") && (i + 3 == n || vowel_letter(at(i + 3)))) { say!(3, "IY", "R"); }
        if starts(i, "ear") { say!(3, "ER"); }
        if starts(i, "air") || ends(i, "are") { say!(3, "EH", "R"); }
        if starts(i, "oor") || starts(i, "ore") || starts(i, "oar") { say!(3, "AO", "R"); }
        if starts(i, "our") { say!(3, "AW", "ER"); }
        if ends(i, "ire") { say!(3, "AY", "ER"); }
        if ends(i, "ure") { say!(3, "UH", "R"); }
        if starts(i, "or") && !vowel_letter(at(i + 2)) {
            if prev == 'w' { say!(2, "ER"); }
            if i + 2 == n && many { say!(2, "ER"); }
            say!(2, "AO", "R");
        }
        if starts(i, "ar") && !vowel_letter(at(i + 2)) {
            if prev == 'w' { say!(2, "AO", "R"); }
            if i + 2 == n && many { say!(2, "ER"); }
            say!(2, "AA", "R");
        }
        if (starts(i, "er") || starts(i, "ir") || starts(i, "ur")) && !vowel_letter(at(i + 2)) && at(i + 2) != 'r' { say!(2, "ER"); }
        // 两个元音字母。
        let two: Option<&[&'static str]> = match (c, next) {
            ('e', 'e') => Some(&["IY"]),
            ('e', 'a') if starts(i, "ead") && i + 3 == n => Some(&["EH"]),
            ('e', 'a') if starts(i, "ealth") || starts(i, "eath") && !ends(i, "eathe") => Some(&["EH"]),
            ('e', 'a') => Some(&["IY"]),
            ('o', 'o') if starts(i, "ook") => Some(&["UH"]),
            ('o', 'o') => Some(&["UW"]),
            ('o', 'u') => Some(&["AW"]),
            ('o', 'w') if ends(i, "own") || starts(i, "owl") || starts(i, "owd") => Some(&["AW"]),
            ('o', 'w') if starts(i, "ower") => Some(&["AW"]),
            ('o', 'w') => Some(&["OW"]),
            ('a', 'i') | ('a', 'y') => Some(&["EY"]),
            ('o', 'a') => Some(&["OW"]),
            ('o', 'i') | ('o', 'y') => Some(&["OY"]),
            ('a', 'u') | ('a', 'w') => Some(&["AO"]),
            ('e', 'w') => Some(&["UW"]),
            ('u', 'e') if i + 2 == n => Some(&["UW"]),
            ('u', 'i') if prev != 'g' && prev != 'q' => Some(&["UW"]),
            ('i', 'e') if i + 2 == n && !many => Some(&["AY"]),
            ('i', 'e') if ends(i, "ied") || ends(i, "ies") => Some(if many { &["IY"] } else { &["AY"] }),
            ('i', 'e') => Some(&["IY"]),
            ('e', 'i') if prev == 'c' => Some(&["IY"]),
            ('e', 'i') => Some(&["EY"]),
            ('e', 'y') if i + 2 == n => Some(&["IY"]),
            ('e', 'u') => Some(&["Y", "UW"]),
            _ => None,
        };
        if let Some(p) = two {
            out.extend_from_slice(p);
            i += 2;
            continue;
        }
        // al 的几种读法。
        if c == 'a' && (starts(i, "alk")) { say!(2, "AO"); }
        if c == 'a' && ((starts(i, "all") && !vowel_letter(at(i + 3))) || starts(i, "alt") || starts(i, "ald")) { say!(1, "AO"); }
        if c == 'a' && starts(i, "alm") { say!(2, "AA"); }
        // 后面是 ld、nd、st 的长元音（child、find、old、most）。
        if c == 'i' && (ends(i, "ild") || ends(i, "ind")) { say!(1, "AY"); }
        if c == 'o' && (starts(i, "old") || starts(i, "olt") || ends(i, "oll") || ends(i, "ost")) { say!(1, "OW"); }
        // 词尾不发音的 e 让前面的元音读长音（make、time、home）；辅音 + le、辅音 + y 之前也是（table、lady）。
        let single = !vowel_letter(next) && next != ' ' && next != 'x';
        let magic = single && ((at(i + 2) == 'e' && i + 3 == n) || (starts(i + 2, "le") && i + 4 == n) || (at(i + 2) == 'y' && i + 3 == n && !prev_v));
        // 词中间"元音 + 一个辅音 + 元音"：o、u 多半读长音（open、music），a、e、i 多半读短音（river、seven）。
        // tion 前面的 a、o、u 读长音（nation、motion）。
        let before_tion = "aou".contains(c) && starts(i + 1, "tion");
        let open = single && vowel_letter(at(i + 2)) && i + 3 < n && "ou".contains(c) && !prev_v && next != 'r' && next != 'v';
        if "aeiou".contains(c) && !prev_v && (magic || open || before_tion) {
            match c {
                'a' => out.push("EY"),
                'e' => out.push("IY"),
                'i' => out.push("AY"),
                'o' => out.push("OW"),
                _ => {
                    if i == 0 || "bcfhkmp".contains(prev) { out.push("Y"); }
                    out.push("UW");
                }
            }
            i += 1;
            continue;
        }
        // 词尾的单个元音。
        if i + 1 == n {
            match c {
                'e' if out.iter().any(|p| is_vowel(p)) => { i += 1; continue; }
                'e' => say!(1, "IY"),
                'y' if out.iter().any(|p| is_vowel(p)) => say!(1, "IY"),
                'y' => say!(1, "AY"),
                'o' => say!(1, "OW"),
                'a' => say!(1, "AX"),
                'i' => say!(1, "IY"),
                'u' => say!(1, "UW"),
                _ => {}
            }
        }
        // 不重读的常见结尾和开头。
        if many && i > 0 && !prev_v {
            if ends(i, "en") || ends(i, "on") || ends(i, "an") { say!(1, "AX"); }
            if ends(i, "el") || ends(i, "al") { say!(1, "AX"); }
            if ends(i, "ent") || ends(i, "ence") || ends(i, "ance") || ends(i, "ant") { say!(1, "AX"); }
            if ends(i, "et") || ends(i, "est") || ends(i, "age") || ends(i, "ive") { say!(1, "IH"); }
            if ends(i, "om") || ends(i, "um") || ends(i, "us") { say!(1, "AX"); }
        }
        if many && i == 0 && c == 'a' && !vowel_letter(next) && vowel_letter(at(2)) && next != 'n' && next != 'f' { say!(1, "AX"); }
        // w 后面的 a 读 o（want、wash）。
        if c == 'a' && prev == 'w' && !vowel_letter(next) && next != ' ' && !"xvkgy".contains(next) { say!(1, "AA"); }
        // 其余：短元音。
        match c {
            'a' => say!(1, "AE"),
            'e' => {
                // 词尾 es、ed 之前不发音的 e 已经在去词尾时处理了；这里是词中间的 e。
                say!(1, "EH")
            }
            'i' | 'y' => say!(1, "IH"),
            'o' => say!(1, "AA"),
            'u' => say!(1, "AH"),
            _ => { i += 1; }
        }
    }
    out
}

/// 能在一个音节开头连着出现的两三个辅音（英文的）。
fn legal_onset(c: &[&str]) -> bool {
    match c {
        [] => true,
        [a] => *a != "NG",
        [a, b] => {
            (["P", "B", "K", "G", "F"].contains(a) && ["L", "R"].contains(b))
                || (["T", "D", "TH", "SH"].contains(a) && *b == "R")
                || (["T", "D", "K", "G", "S", "TH"].contains(a) && *b == "W")
                || (*a == "S" && ["P", "T", "K", "M", "N", "L", "F"].contains(b))
                || (*b == "Y" && ["P", "B", "K", "G", "F", "V", "M", "N", "HH", "T", "D"].contains(a))
        }
        [a, b, c] => *a == "S" && ["P", "T", "K"].contains(b) && legal_onset(&[b, c]),
        _ => false,
    }
}

/// 把一串音素分成音节：每个元音一个，元音之间的辅音尽量多归后一个音节（要是英文里能开头的组合）。
pub fn syllabify(p: &[&'static str]) -> Vec<Vec<&'static str>> {
    let nuclei: Vec<usize> = (0..p.len()).filter(|&i| is_vowel(p[i])).collect();
    if nuclei.len() <= 1 {
        return if p.is_empty() { Vec::new() } else { vec![p.to_vec()] };
    }
    let mut out = Vec::new();
    let mut start = 0;
    for w in nuclei.windows(2) {
        let between = &p[w[0] + 1..w[1]];
        let split = (0..=between.len()).find(|&j| legal_onset(&between[j..])).unwrap_or(between.len());
        let cut = w[0] + 1 + split;
        out.push(p[start..cut].to_vec());
        start = cut;
    }
    out.push(p[start..].to_vec());
    out
}

/// 一个音符的歌词 → 它要唱的各个音节（[`crate::singer::syllable`] 认的写法）。
/// 音素串（`S-T-R-IY-T`）、拼音（`english` 为 false 时）原样是一个音节；英文词按上面的办法读出来、分好音节，
/// 每个音节写成一个音素串。
pub fn syllables(text: &str, english: bool) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let dashed = text.replace('.', "-");
    let spelled = dashed.chars().all(|c| c.is_ascii_uppercase() || c == '-') && dashed.chars().any(|c| c.is_ascii_uppercase());
    if spelled {
        let text = dashed.as_str();
        // 用户自己写的音素：一个音符上写了几个元音就是几个音节。
        let tokens: Option<Vec<&'static str>> = text.split('-').filter(|t| !t.is_empty()).map(|t| VOWELS.iter().chain(CONSONANTS.iter()).find(|k| **k == t).copied()).collect();
        if let Some(tokens) = tokens {
            return syllabify(&tokens).iter().map(|s| s.join("-")).collect();
        }
    }
    if !english && crate::singer::is_pinyin(text) {
        return vec![text.to_string()];
    }
    syllabify(&phonemes(text)).iter().map(|s| s.join("-")).collect()
}

/// 一条歌声音轨的音符：把每个词的音节分到音符上。一个音符唱一个音节；歌词是 `+` 的音符接着唱前面那个词的
/// 下一个音节；词的音节比给它的音符多时，多出来的挤在最后一个音符里（把它等分成几个同音高的音）。
/// 拼音的字原样不动；`english` 为 true 时所有的词都按英文读（"she"、"me"、"long" 这些也是拼音的写法）。
pub fn spread(notes: &[muser_core::Note], english: bool) -> Vec<muser_core::Note> {
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].start.0.total_cmp(&notes[b].start.0));
    let mut out: Vec<muser_core::Note> = Vec::with_capacity(notes.len());
    let mut left: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    // 上一个词还剩几个音节没有音符唱：把最后那个音符等分。
    let squeeze = |out: &mut Vec<muser_core::Note>, left: &mut std::collections::VecDeque<String>| {
        if left.is_empty() {
            return;
        }
        let Some(last) = out.pop() else {
            left.clear();
            return;
        };
        let parts = left.len() + 1;
        let each = last.length.0 / parts as f64;
        let first = last.lyric.clone();
        for (k, lyric) in std::iter::once(first).chain(left.drain(..).map(Some)).enumerate() {
            let mut n = last.clone();
            n.start = muser_core::Beats(last.start.0 + k as f64 * each);
            n.length = muser_core::Beats(each);
            n.lyric = lyric;
            // 换气、滑音只在这个词的第一个音节上。
            if k > 0 {
                n.breath = false;
                n.slide = false;
            }
            if k + 1 < parts {
                n.slide_out = false;
            }
            out.push(n);
        }
    };
    let mut vowel: Option<String> = None;
    for i in order {
        let mut n = notes[i].clone();
        match n.lyric.as_deref().map(str::trim) {
            Some("+") => {
                n.lyric = left.pop_front().or_else(|| {
                    // 没有音节可接了：上一个音节拖到这个音上（一个音节唱几个音）。元音接着唱，
                    // 元音后面的辅音挪到这个音的末尾（"sing" 拖两个音是 si—ing，不是 sing—i）。
                    let held = vowel.clone()?;
                    let before = out.last_mut()?;
                    let text = before.lyric.clone()?;
                    let parts: Vec<&str> = text.split('-').collect();
                    match parts.iter().position(|p| is_vowel(p)) {
                        Some(at) if at + 1 < parts.len() => {
                            before.lyric = Some(parts[..=at].join("-"));
                            Some(std::iter::once(held.as_str()).chain(parts[at + 1..].iter().copied()).collect::<Vec<_>>().join("-"))
                        }
                        _ => Some(held),
                    }
                });
            }
            Some(text) if !text.is_empty() => {
                squeeze(&mut out, &mut left);
                let mut syl: std::collections::VecDeque<String> = syllables(text, english).into();
                n.lyric = syl.pop_front().or(n.lyric);
                left = syl;
            }
            _ => squeeze(&mut out, &mut left),
        }
        if let Some(l) = n.lyric.as_deref() {
            vowel = l.split('-').find(|p| is_vowel(p)).map(str::to_string).or(vowel);
        }
        out.push(n);
    }
    squeeze(&mut out, &mut left);
    out
}

const CONSONANTS: [&str; 25] = ["P", "B", "T", "D", "K", "G", "CH", "JH", "F", "V", "TH", "DH", "S", "Z", "SH", "ZH", "HH", "H", "M", "N", "NG", "L", "R", "W", "Y"];

#[cfg(test)]
mod tests {
    use super::*;

    fn say(w: &str) -> String {
        phonemes(w).join(" ")
    }

    #[test]
    fn common_words_are_read_by_the_rules() {
        let mut wrong = Vec::new();
        for (w, p) in [
            ("street", "S T R IY T"), ("night", "N AY T"), ("light", "L AY T"), ("time", "T AY M"), ("home", "HH OW M"), ("make", "M EY K"), ("rain", "R EY N"), ("day", "D EY"),
            ("sing", "S IH NG"), ("song", "S AA NG"), ("moon", "M UW N"), ("star", "S T AA R"), ("dream", "D R IY M"), ("sky", "S K AY"), ("fly", "F L AY"), ("happy", "HH AE P IY"),
            ("down", "D AW N"), ("show", "SH OW"), ("cold", "K OW L D"), ("find", "F AY N D"), ("table", "T EY B AX L"), ("nation", "N EY SH AX N"), ("phone", "F OW N"), ("quick", "K W IH K"),
            ("thing", "TH IH NG"), ("think", "TH IH NG K"), ("fox", "F AA K S"), ("jump", "JH AH M P"), ("city", "S IH T IY"), ("gentle", "JH EH N T AX L"), ("bird", "B ER D"), ("morning", "M AO R N IH NG"),
        ] {
            if say(w) != p {
                wrong.push(format!("{w}: {} (want {p})", say(w)));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    #[ignore]
    fn print_words() {
        let text = std::env::var("WORDS").unwrap_or_default();
        for w in text.split_whitespace() {
            println!("{w:12} {}   | {}", say(w), syllables(w, true).join(" "));
        }
    }

    #[test]
    fn endings_are_added_to_the_stem() {
        assert_eq!(say("loved"), "L AH V D");
        assert_eq!(say("wanted"), "W AA N T IH D");
        assert_eq!(say("jumped"), "JH AH M P T");
        assert_eq!(say("making"), "M EY K IH NG");
        assert_eq!(say("running"), "R AH N IH NG");
        assert_eq!(say("singing"), "S IH NG IH NG");
        assert_eq!(say("stars"), "S T AA R Z");
        assert_eq!(say("cats"), "K AE T S");
        assert_eq!(say("wishes"), "W IH SH IH Z");
        assert_eq!(say("slowly"), "S L OW L IY");
    }

    #[test]
    fn words_split_into_syllables() {
        let syl = |w: &str| syllables(w, true).join(" ");
        assert_eq!(syl("beautiful"), "B-Y-UW T-IH F-AX-L");
        assert_eq!(syl("tonight"), "T-AX N-AY-T");
        assert_eq!(syl("street"), "S-T-R-IY-T");
        assert_eq!(syl("morning"), "M-AO-R N-IH-NG");
        assert_eq!(syl("extra"), "EH-K S-T-R-AX");
        // 用户写的音素原样用；拼音在拼音模式下不动，在英文模式下按英文读。
        assert_eq!(syl("HH-AX-L-OW"), "HH-AX L-OW");
        assert_eq!(syl("S.T.R.IY.T"), "S-T-R-IY-T");
        assert_eq!(syllables("ming", false), vec!["ming"]);
        assert_eq!(syllables("love", false), vec!["L-AH-V"]);
        assert_eq!(syllables("she", true), vec!["SH-IY"]);
    }

    #[test]
    fn syllables_are_spread_over_the_notes() {
        let note = |start: f64, lyric: &str| {
            let mut n = muser_core::Note::new(60, start, 1.0);
            n.lyric = Some(lyric.into());
            n
        };
        let sung = |notes: &[muser_core::Note]| spread(notes, true).iter().map(|n| format!("{}@{}", n.lyric.clone().unwrap_or_default(), n.start.0)).collect::<Vec<_>>().join(" ");
        // + 接着唱下一个音节；没有音节了就拖着元音（词尾的辅音挪到最后）。
        assert_eq!(sung(&[note(0.0, "beautiful"), note(1.0, "+"), note(2.0, "+"), note(3.0, "+")]), "B-Y-UW@0 T-IH@1 F-AX@2 AX-L@3");
        // 音符不够：多出来的音节挤在最后一个音符里。
        assert_eq!(sung(&[note(0.0, "beautiful"), note(1.0, "+"), note(2.0, "day")]), "B-Y-UW@0 T-IH@1 F-AX-L@1.5 D-EY@2");
        assert_eq!(sung(&[note(0.0, "tonight")]), "T-AX@0 N-AY-T@0.5");
        // 一个音节拖几个音：辅音收在最后。
        assert_eq!(sung(&[note(0.0, "sing"), note(1.0, "+"), note(2.0, "+")]), "S-IH@0 IH@1 IH-NG@2");
        // 拼音不动。
        assert_eq!(spread(&[note(0.0, "ming"), note(1.0, "yue")], false).iter().map(|n| n.lyric.clone().unwrap()).collect::<Vec<_>>(), vec!["ming", "yue"]);
    }
}
