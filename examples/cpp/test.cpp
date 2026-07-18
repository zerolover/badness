#include <stdexcept>
#include <string>

#include "badness.hpp"
#include "utest.h"

namespace {

badness::SyntaxTree parse(std::u16string source) {
    BadnessTree* raw = nullptr;
    const BadnessStatus status = badness_parse_utf16(
        reinterpret_cast<const uint16_t*>(source.data()), source.size(), &raw);
    if (status != BADNESS_OK) {
        throw std::runtime_error("badness_parse_utf16 failed");
    }

    badness::SyntaxTree tree(raw, std::move(source));
    badness_tree_free(raw);
    return tree;
}

} // namespace

UTEST(SyntaxTree, EmptyInput) {
    const auto tree = parse(u"");

    ASSERT_EQ(1u, tree.nodeCount());
    ASSERT_EQ(tree.root(), 0u);
    ASSERT_TRUE(tree.children(tree.root()).empty());
    ASSERT_TRUE(tree.text(tree.root()).empty());
    ASSERT_TRUE(tree.errors().empty());
}

UTEST(SyntaxTree, StructureNavigation) {
    const auto tree = parse(u"a😀\\begin{equation}x^2\\end{equation}{unclosed");
    const badness::NodeId root = tree.root();

    ASSERT_FALSE(tree.parent(root));
    const auto rootChildren = tree.children(root);
    ASSERT_EQ(1u, rootChildren.size());

    const badness::NodeId paragraph = rootChildren.front();
    const auto paragraphParent = tree.parent(paragraph);
    ASSERT_TRUE(paragraphParent);
    ASSERT_EQ(root, *paragraphParent);

    const auto ancestors = tree.ancestors(paragraph);
    ASSERT_EQ(2u, ancestors.size());
    ASSERT_EQ(paragraph, ancestors[0]);
    ASSERT_EQ(root, ancestors[1]);

    const auto topLevel = tree.children(paragraph);
    ASSERT_EQ(3u, topLevel.size());
    ASSERT_FALSE(tree.prevSiblingOrToken(topLevel[0]));
    const auto firstNext = tree.nextSiblingOrToken(topLevel[0]);
    const auto secondPrev = tree.prevSiblingOrToken(topLevel[1]);
    const auto secondNext = tree.nextSiblingOrToken(topLevel[1]);
    const auto thirdPrev = tree.prevSiblingOrToken(topLevel[2]);
    ASSERT_TRUE(firstNext);
    ASSERT_TRUE(secondPrev);
    ASSERT_TRUE(secondNext);
    ASSERT_TRUE(thirdPrev);
    ASSERT_EQ(topLevel[1], *firstNext);
    ASSERT_EQ(topLevel[0], *secondPrev);
    ASSERT_EQ(topLevel[2], *secondNext);
    ASSERT_EQ(topLevel[1], *thirdPrev);
    ASSERT_FALSE(tree.nextSiblingOrToken(topLevel[2]));

    const auto descendants = tree.descendants(root);
    ASSERT_EQ(tree.nodeCount(), descendants.size());
    for (badness::NodeId id = 0; id < tree.nodeCount(); ++id) {
        ASSERT_EQ(id, descendants[id]);
    }
}

UTEST(SyntaxTree, SubtreeAndMixedSiblingNavigation) {
    const auto tree = parse(u"a😀\\begin{equation}x^2\\end{equation}{unclosed");
    const badness::NodeId paragraph = tree.children(tree.root()).front();
    const badness::NodeId environment = tree.children(paragraph)[1];
    const auto environmentChildren = tree.children(environment);
    const badness::NodeId begin = environmentChildren[0];
    const badness::NodeId math = environmentChildren[1];
    const badness::NodeId end = environmentChildren[2];

    const auto subtree = tree.descendants(environment);
    ASSERT_EQ(19u, subtree.size());
    ASSERT_EQ(environment, subtree.front());
    ASSERT_EQ(begin, subtree[1]);
    ASSERT_EQ(end, subtree[13]);
    const auto mathParent = tree.parent(math);
    ASSERT_TRUE(mathParent);
    ASSERT_EQ(environment, *mathParent);

    const auto beginChildren = tree.children(begin);
    ASSERT_EQ(2u, beginChildren.size());
    const badness::NodeId controlWord = beginChildren[0];
    const badness::NodeId nameGroup = beginChildren[1];
    ASSERT_TRUE(tree.isToken(controlWord));
    ASSERT_FALSE(tree.isToken(nameGroup));
    ASSERT_TRUE(tree.children(controlWord).empty());
    ASSERT_FALSE(tree.prevSiblingOrToken(controlWord));

    const auto controlWordNext = tree.nextSiblingOrToken(controlWord);
    const auto nameGroupPrev = tree.prevSiblingOrToken(nameGroup);
    ASSERT_TRUE(controlWordNext);
    ASSERT_TRUE(nameGroupPrev);
    ASSERT_EQ(nameGroup, *controlWordNext);
    ASSERT_EQ(controlWord, *nameGroupPrev);
    ASSERT_FALSE(tree.nextSiblingOrToken(nameGroup));

    const badness::NodeId leftBrace = tree.children(nameGroup).front();
    const auto controlWordTokenNext = tree.nextToken(controlWord);
    const auto controlWordTokenPrev = tree.prevToken(controlWord);
    ASSERT_TRUE(controlWordTokenNext);
    ASSERT_TRUE(controlWordTokenPrev);
    ASSERT_EQ(leftBrace, *controlWordTokenNext);
    ASSERT_EQ(tree.children(paragraph).front(), *controlWordTokenPrev);
}

UTEST(SyntaxTree, SourceTextAndDiagnostics) {
    const std::u16string source = u"a😀\\begin{equation}x^2\\end{equation}{unclosed";
    const auto tree = parse(source);

    ASSERT_TRUE(tree.source() == source);
    ASSERT_TRUE(tree.text(tree.root()) == source);
    ASSERT_EQ(1u, tree.errors().size());
    ASSERT_EQ(36u, tree.errors()[0].start);
    ASSERT_EQ(37u, tree.errors()[0].end);
    ASSERT_STREQ("unclosed `{`", tree.errors()[0].message.c_str());
}

UTEST(SyntaxTree, Utf16OffsetsAndValidUnicodeInput) {
    const auto invalidTree = parse(u"a😀\\begin{equation}x^2\\end{equation}{unclosed");
    const badness::NodeId paragraph = invalidTree.children(invalidTree.root()).front();
    const auto topLevel = invalidTree.children(paragraph);
    ASSERT_EQ(0u, invalidTree.start(topLevel[0]));
    ASSERT_EQ(3u, invalidTree.end(topLevel[0]));
    ASSERT_EQ(3u, invalidTree.start(topLevel[1]));
    ASSERT_EQ(36u, invalidTree.end(topLevel[1]));
    const auto emoji = invalidTree.tokenAt(2);
    ASSERT_TRUE(emoji.left);
    ASSERT_TRUE(emoji.right);
    ASSERT_EQ(topLevel[0], *emoji.left);
    ASSERT_EQ(topLevel[0], *emoji.right);

    const std::u16string source = u"$x + \\text{中文}$";
    const auto validTree = parse(source);
    ASSERT_TRUE(validTree.errors().empty());
    ASSERT_EQ(15u, validTree.end(validTree.root()));
    ASSERT_TRUE(validTree.text(validTree.root()) == source);
}

UTEST(SyntaxTree, TokenNavigationIncludesTrivia) {
    const auto tree = parse(u"foo bar");
    const badness::NodeId paragraph = tree.children(tree.root()).front();
    const auto tokens = tree.children(paragraph);
    ASSERT_EQ(3u, tokens.size());
    ASSERT_TRUE(tree.isToken(tokens[0]));
    ASSERT_TRUE(tree.isTrivia(tokens[1]));
    ASSERT_TRUE(tree.isToken(tokens[2]));

    ASSERT_FALSE(tree.prevToken(tokens[0]));
    const auto firstNext = tree.nextToken(tokens[0]);
    const auto spacePrev = tree.prevToken(tokens[1]);
    const auto spaceNext = tree.nextToken(tokens[1]);
    const auto lastPrev = tree.prevToken(tokens[2]);
    ASSERT_TRUE(firstNext);
    ASSERT_TRUE(spacePrev);
    ASSERT_TRUE(spaceNext);
    ASSERT_TRUE(lastPrev);
    ASSERT_EQ(tokens[1], *firstNext);
    ASSERT_EQ(tokens[0], *spacePrev);
    ASSERT_EQ(tokens[2], *spaceNext);
    ASSERT_EQ(tokens[1], *lastPrev);
    ASSERT_FALSE(tree.nextToken(tokens[2]));
}

UTEST(SyntaxTree, TokenAtOffset) {
    const auto tree = parse(u"foo bar");
    const badness::NodeId paragraph = tree.children(tree.root()).front();
    const auto tokens = tree.children(paragraph);

    const auto start = tree.tokenAt(0);
    const auto insideLeft = tree.tokenAt(1);
    const auto leftBoundary = tree.tokenAt(3);
    const auto rightBoundary = tree.tokenAt(4);
    const auto insideRight = tree.tokenAt(6);
    const auto end = tree.tokenAt(7);
    const auto outside = tree.tokenAt(8);
    ASSERT_TRUE(start.left);
    ASSERT_TRUE(start.right);
    ASSERT_TRUE(insideLeft.left);
    ASSERT_TRUE(insideLeft.right);
    ASSERT_TRUE(leftBoundary.left);
    ASSERT_TRUE(leftBoundary.right);
    ASSERT_TRUE(rightBoundary.left);
    ASSERT_TRUE(rightBoundary.right);
    ASSERT_TRUE(insideRight.left);
    ASSERT_TRUE(insideRight.right);
    ASSERT_TRUE(end.left);
    ASSERT_TRUE(end.right);
    ASSERT_EQ(tokens[0], *start.left);
    ASSERT_EQ(tokens[0], *start.right);
    ASSERT_EQ(tokens[0], *insideLeft.left);
    ASSERT_EQ(tokens[0], *insideLeft.right);
    ASSERT_EQ(tokens[0], *leftBoundary.left);
    ASSERT_EQ(tokens[1], *leftBoundary.right);
    ASSERT_EQ(tokens[1], *rightBoundary.left);
    ASSERT_EQ(tokens[2], *rightBoundary.right);
    ASSERT_EQ(tokens[2], *insideRight.left);
    ASSERT_EQ(tokens[2], *insideRight.right);
    ASSERT_EQ(tokens[2], *end.left);
    ASSERT_EQ(tokens[2], *end.right);
    ASSERT_FALSE(outside.left);
    ASSERT_FALSE(outside.right);

    const auto empty = parse(u"").tokenAt(0);
    ASSERT_FALSE(empty.left);
    ASSERT_FALSE(empty.right);
}

UTEST_MAIN()
