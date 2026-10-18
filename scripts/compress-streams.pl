#!/usr/bin/env perl
# A second writer of Unix compress streams, for the decoder's fixtures (C-015): LZW as ncompress writes it (codes
# little-endian in groups of eight, the whole group written when the width grows or the table is cleared), over a
# deterministic text from a linear congruential generator, so the fixtures can be made again. Written in the
# fourth refuter pass; gzip -dc and the real 7-Zip read every stream it made back to the same bytes.
#
#   perl scripts/compress-streams.pl tests/fixtures/clear12.raw tests/fixtures/clear12.Z clear 24000 12
#   perl scripts/compress-streams.pl tests/fixtures/plain.raw tests/fixtures/plain.Z nonblock 6000 16
#   perl scripts/compress-streams.pl tests/fixtures/sixteen.raw tests/fixtures/sixteen.Z noclear 80000 16
#
# "clear" sends a clear code 1003 codes after the table fills (ncompress sends one when its ratio drops); "noclear"
# never does; "nonblock" writes the old format without block mode, where 256 is an ordinary code.
# args: out_raw out_Z mode(clear|noclear|nonblock) size maxbits
my ($raw,$zf,$mode,$size,$maxbits)=@ARGV;
my $seed=12345; my $data='';
my @alpha=split //,"abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123456789 \n.,";
while(length($data)<$size){ $seed=($seed*1103515245+12345)%2147483648; $data.=$alpha[($seed>>16)%scalar(@alpha)]; }
open my $f,'>:raw',$raw or die; print $f $data; close $f;
my $block = $mode ne 'nonblock';
my $mmc=1<<$maxbits;
my @out=(0x1f,0x9d,($block?0x80:0)|$maxbits);
my ($bits,$count,$nb,$in_group)=(0,0,9,0);
my $maxcode=(1<<$nb)-1;
my $first = $block?257:256;
my $free=$first; my %dict;
my @b=unpack('C*',$data);
my $ent=shift @b;
my $emit=sub{ my($code)=@_; $bits|=$code<<$count; $count+=$nb; while($count>=8){push @out,$bits&0xff;$bits>>=8;$count-=8;} $in_group=($in_group+1)%8; };
my $clears=0; my $fullcodes=0;
for my $c (@b){
  if(exists $dict{"$ent,$c"}){ $ent=$dict{"$ent,$c"}; next; }
  $emit->($ent);
  if($free>$maxcode){ while($in_group!=0){$emit->(0);} $nb++; $maxcode = $nb==$maxbits ? $mmc : (1<<$nb)-1; }
  if($free<$mmc){ $dict{"$ent,$c"}=$free; $free++; }
  elsif($mode eq 'clear'){ $fullcodes++; if($fullcodes>=1003){ $emit->(256); while($in_group!=0){$emit->(0);} $nb=9; $maxcode=511; $free=257; %dict=(); $clears++; $fullcodes=0; } }
  $ent=$c;
}
$emit->($ent);
push @out,$bits&0xff if $count>0;
open my $g,'>:raw',$zf or die; print $g pack('C*',@out); close $g;
print STDERR "bytes ",scalar(@out)," clears $clears final nbits $nb\n";
