<?php

declare(strict_types=1);

function readRoute(string $url): string
{
    $context = stream_context_create([
        'http' => [
            'ignore_errors' => true,
            'timeout' => 30,
        ],
    ]);
    $body = file_get_contents($url, false, $context);
    if ($body === false || !isset($http_response_header[0])) {
        fwrite(STDERR, "Published route returned no HTTP response.\n");
        exit(1);
    }
    if (!str_contains($http_response_header[0], ' 200 ')) {
        fwrite(STDERR, "{$http_response_header[0]}\n");
        exit(1);
    }
    return $body;
}

$status = json_decode(readRoute('http://127.0.0.1:18443/status.php'), true);
if (!is_array($status) || ($status['installed'] ?? false) !== true) {
    fwrite(STDERR, "Published Nextcloud status is not installed.\n");
    exit(1);
}
$second = readRoute('http://127.0.0.1:18443/second/');
trim($second) === 'second-application' || exit(1);
