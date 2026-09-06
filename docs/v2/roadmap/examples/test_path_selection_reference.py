import unittest
from path_selection_reference import PathBinding, admits_mode, select_paths


class SelectorReferenceTests(unittest.TestCase):
    def setUp(self):
        self.one = PathBinding((1, 4), (10,), "one")
        self.two_a = PathBinding((1, 2, 4), (11, 12), "two-a")
        self.two_b = PathBinding((1, 3, 4), (13, 14), "two-b")
        self.three = PathBinding((1, 2, 3, 4), (11, 15, 14), "three")
        self.other = PathBinding((5, 6, 7, 8), (20, 21, 22), "other")
        self.paths = [self.three, self.two_b, self.one, self.two_a]

    def test_endpoint_partitions_do_not_share_global_minimum(self):
        got = select_paths([self.other, self.one], "all_shortest")
        self.assertEqual(got, [self.one, self.other])

    def test_qualification_precedes_selection(self):
        got = select_paths(self.paths, "all_shortest", qualifies=lambda p: p.length > 1)
        self.assertEqual(got, [self.two_a, self.two_b])

    def test_all_shortest_keeps_ties(self):
        self.assertEqual(select_paths([self.three, self.two_b, self.two_a], "all_shortest"),
                         [self.two_a, self.two_b])

    def test_counted_shortest_can_cross_length_groups(self):
        self.assertEqual(select_paths(self.paths, "counted_shortest", count=3),
                         [self.one, self.two_a, self.two_b])

    def test_counted_shortest_can_select_one_tie(self):
        self.assertEqual(select_paths([self.two_b, self.two_a], "counted_shortest", count=1),
                         [self.two_a])

    def test_shortest_groups_include_complete_groups(self):
        self.assertEqual(select_paths(self.paths, "shortest_groups", count=2),
                         [self.one, self.two_a, self.two_b])

    def test_count_and_group_count_differ(self):
        shortest = select_paths(self.paths, "counted_shortest", count=2)
        groups = select_paths(self.paths, "shortest_groups", count=2)
        self.assertEqual(len(shortest), 2)
        self.assertEqual(len(groups), 3)

    def test_zero_count_is_empty_per_partition(self):
        for selection in ("any", "counted_shortest", "shortest_groups"):
            with self.subTest(selection=selection):
                self.assertEqual(select_paths(self.paths + [self.other], selection, count=0), [])

    def test_count_greater_than_available(self):
        self.assertEqual(len(select_paths(self.paths, "counted_shortest", count=99)), 4)

    def test_empty_input_has_no_partitions(self):
        self.assertEqual(select_paths([], "all_shortest"), [])

    def test_all_rejected_has_no_partitions(self):
        self.assertEqual(select_paths(self.paths, "all_shortest", qualifies=lambda p: False), [])

    def test_any_count_applies_per_partition(self):
        got = select_paths(self.paths + [self.other], "any", count=1)
        self.assertEqual(got, [self.one, self.other])

    def test_zero_edge_path_has_one_endpoint_node(self):
        p = PathBinding((1,), ())
        self.assertEqual(p.length, 0)
        self.assertEqual(p.endpoints, (1, 1))
        for mode in ("walk", "trail", "simple", "acyclic"):
            self.assertTrue(admits_mode(p, mode))

    def test_simple_does_not_imply_trail(self):
        p = PathBinding((1, 2, 1), (10, 10))  # Traverse one undirected edge twice.
        self.assertTrue(admits_mode(p, "simple"))
        self.assertFalse(admits_mode(p, "trail"))
        self.assertFalse(admits_mode(p, "acyclic"))
        self.assertTrue(admits_mode(p, "walk"))

    def test_parallel_edges_can_form_a_trail_cycle(self):
        p = PathBinding((1, 2, 1), (10, 11))
        self.assertTrue(admits_mode(p, "trail"))
        self.assertTrue(admits_mode(p, "simple"))
        self.assertFalse(admits_mode(p, "acyclic"))

    def test_simple_rejects_an_interior_repeat(self):
        self.assertFalse(admits_mode(PathBinding((1, 2, 3, 2, 4), (10, 11, 12, 13)), "simple"))

    def test_simple_cannot_close_then_continue(self):
        self.assertFalse(admits_mode(PathBinding((1, 2, 1, 3), (10, 11, 12)), "simple"))

    def test_one_loop_is_simple_and_trail_but_not_acyclic(self):
        p = PathBinding((1, 1), (10,))
        self.assertTrue(admits_mode(p, "simple"))
        self.assertTrue(admits_mode(p, "trail"))
        self.assertFalse(admits_mode(p, "acyclic"))

    def test_distinct_binding_tags_are_not_deduplicated(self):
        other_binding = PathBinding(self.one.nodes, self.one.edges, "another-binding")
        got = select_paths([self.one, other_binding], "all_shortest")
        self.assertEqual(len(got), 2)

    def test_invalid_path_shape_rejected(self):
        with self.assertRaises(ValueError):
            PathBinding((), ())
        with self.assertRaises(ValueError):
            PathBinding((1, 2), ())

    def test_invalid_count_and_selection_rejected(self):
        for count in (-1, None, 1.5, True):
            with self.subTest(count=count), self.assertRaises(ValueError):
                select_paths(self.paths, "any", count=count)
        with self.assertRaises(ValueError):
            select_paths(self.paths, "all_shortest", count=1)
        with self.assertRaises(ValueError):
            select_paths(self.paths, "unknown")
        with self.assertRaises(ValueError):
            admits_mode(self.one, "unknown")

    def test_input_order_does_not_change_reference_tie_policy(self):
        self.assertEqual(select_paths(self.paths, "counted_shortest", count=2),
                         select_paths(reversed(self.paths), "counted_shortest", count=2))


if __name__ == "__main__":
    unittest.main()
