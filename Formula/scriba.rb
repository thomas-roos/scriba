class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.10.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.10.0/scriba-x86_64-apple-darwin"
    sha256 "eb738c2cb121ece7e3b6848a13cc6c1c0808b8c125403f86650ae5475b5cb749"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.10.0/scriba-aarch64-apple-darwin"
    sha256 "39a07c0e8af221b0844f5a290d4ec4acde792d7b4cafe761d66ca63d4105af21"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
