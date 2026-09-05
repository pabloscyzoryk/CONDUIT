import unittest
from prepare_exact_neighbors8 import local_distance


class NeighborDistanceTests(unittest.TestCase):
    def test_numeric_neighborhood(self):
        a = {'lot_max': 5, 'distance': 10., 'enabled': True}
        b = {**a, 'distance': 12.}
        distance = local_distance(a, b)
        self.assertEqual(distance[0], 1)
        self.assertAlmostEqual(distance[1], 2/12)

    def test_never_changes_broker_or_lot_cap(self):
        self.assertIsNone(local_distance({'lot_max': 5}, {'lot_max': 10}))
        self.assertIsNone(local_distance({'swap_long_points': -75}, {'swap_long_points': -70}))

    def test_enable_or_disable_is_not_local_numeric_stability(self):
        self.assertIsNone(local_distance({'enabled': True}, {'enabled': False}))
        self.assertIsNone(local_distance({'distance': 0.}, {'distance': .1}))
        self.assertIsNone(local_distance({'mode': 'a'}, {'mode': 'b'}))

    def test_remote_and_identical_parameters_are_not_neighbors(self):
        self.assertIsNone(local_distance({'x': 1.}, {'x': 100.}))
        self.assertIsNone(local_distance({'x': 1.}, {'x': 1.}))


if __name__ == '__main__':
    unittest.main()
