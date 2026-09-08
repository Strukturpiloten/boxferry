<?php

declare(strict_types=1);

function request(string $url, string $method, string $authorization, ?string $content = null): array
{
    $headers = [
        'Host: cloud.example.invalid',
        "Authorization: Basic {$authorization}",
        'Connection: close',
    ];
    if ($content !== null) {
        $headers[] = 'Content-Type: text/plain';
    }
    $context = stream_context_create([
        'http' => [
            'method' => $method,
            'header' => implode("\r\n", $headers) . "\r\n",
            'content' => $content ?? '',
            'ignore_errors' => true,
            'timeout' => 30,
        ],
    ]);
    $body = file_get_contents($url, false, $context);
    if ($body === false || !isset($http_response_header[0])) {
        fwrite(STDERR, "WebDAV {$method} returned no HTTP response.\n");
        exit(1);
    }
    if (!preg_match('/^HTTP\/\S+\s+(\d{3})\b/', $http_response_header[0], $matches)) {
        fwrite(STDERR, "WebDAV {$method} returned an invalid status line.\n");
        exit(1);
    }
    return [(int) $matches[1], $body];
}

$password = getenv('BF_PASSWORD');
$payloadValue = getenv('BF_PAYLOAD');
$upload = getenv('BF_UPLOAD');
if ($password === false || $payloadValue === false || $upload === false) {
    fwrite(STDERR, "WebDAV probe environment is incomplete.\n");
    exit(1);
}

$authorization = base64_encode("boxferry-admin:{$password}");
$payload = "{$payloadValue}\n";
$url = 'http://proxy:8080/remote.php/dav/files/boxferry-admin/boxferry-live.txt';
if ($upload === 'true') {
    [$status] = request($url, 'PUT', $authorization, $payload);
    in_array($status, [201, 204], true) || exit(1);
}
[$status, $body] = request($url, 'GET', $authorization);
($status === 200 && hash_equals($payload, $body)) || exit(1);
