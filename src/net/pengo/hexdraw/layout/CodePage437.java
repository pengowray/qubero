/*
 * Created on 27/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

import net.pengo.splash.SimplySizedFont;

/**
 * @author Que
 *
 * TODO To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
public class CodePage437 extends AsciiTileSet {

	char[] codepage = new char[] {
			'\u0000','\u263A','\u263B','\u2665','\u2666','\u2663','\u2660','\u2022','\u25D8','\u25CB','\u25D9','\u2642','\u2640','\u266A','\u266B','\u263C',
			'\u25BA','\u25C4','\u2195','\u203C','\u00B6','\u00A7','\u25AC','\u21A8','\u2191','\u2193','\u2192','\u2190','\u221F','\u2194','\u25B2','\u25BC',
			'\u0020','\u0021','\u0022','\u0023','\u0024','\u0025','\u0026',0x27,'\u0028','\u0029','\u002A','\u002B','\u002C','\u002D','\u002E','\u002F',
			'\u0030','\u0031','\u0032','\u0033','\u0034','\u0035','\u0036','\u0037','\u0038','\u0039','\u003A','\u003B','\u003C','\u003D','\u003E','\u003F',
			'\u0040','\u0041','\u0042','\u0043','\u0044','\u0045','\u0046','\u0047','\u0048','\u0049','\u004A','\u004B','\u004C','\u004D','\u004E','\u004F',
			'\u0050','\u0051','\u0052','\u0053','\u0054','\u0055','\u0056','\u0057','\u0058','\u0059','\u005A','\u005B',0x5C,'\u005D','\u005E','\u005F',
			'\u0060','\u0061','\u0062','\u0063','\u0064','\u0065','\u0066','\u0067','\u0068','\u0069','\u006A','\u006B','\u006C','\u006D','\u006E','\u006F',
			'\u0070','\u0071','\u0072','\u0073','\u0074','\u0075','\u0076','\u0077','\u0078','\u0079','\u007A','\u007B','\u007C','\u007D','\u007E','\u007F',
			'\u00C7','\u00FC','\u00E9','\u00E2','\u00E4','\u00E0','\u00E5','\u00E7','\u00EA','\u00EB','\u00E8','\u00EF','\u00EE','\u00EC','\u00C4','\u00C5',
			'\u00C9','\u00E6','\u00C6','\u00F4','\u00F6','\u00F2','\u00FB','\u00F9','\u00FF','\u00D6','\u00DC','\u00A2','\u00A3','\u00A5','\u20A7','\u0192',
			'\u00E1','\u00ED','\u00F3','\u00FA','\u00F1','\u00D1','\u00AA','\u00BA','\u00BF','\u2310','\u00AC','\u00BD','\u00BC','\u00A1','\u00AB','\u00BB',
			'\u2591','\u2592','\u2593','\u2502','\u2524','\u2561','\u2562','\u2556','\u2555','\u2563','\u2551','\u2557','\u255D','\u255C','\u255B','\u2510',
			'\u2514','\u2534','\u252C','\u251C','\u2500','\u253C','\u255E','\u255F','\u255A','\u2554','\u2569','\u2566','\u2560','\u2550','\u256C','\u2567',
			'\u2568','\u2564','\u2565','\u2559','\u2558','\u2552','\u2553','\u256B','\u256A','\u2518','\u250C','\u2588','\u2584','\u258C','\u2590','\u2580',
			'\u03B1','\u00DF','\u0393','\u03C0','\u03A3','\u03C3','\u00B5','\u03C4','\u03A6','\u0398','\u03A9','\u03B4','\u221E','\u03C6','\u03B5','\u2229',
			'\u2261','\u00B1','\u2265','\u2264','\u2320','\u2321','\u00F7','\u2248','\u00B0','\u2219','\u00B7','\u221A','\u207F','\u00B2','\u25A0','\u00A0'
	};


	public CodePage437(SimplySizedFont font, boolean antialias) {
		super(font, antialias);
	}

    protected char tileChar(int tile) {
		return codepage[tile & 0xff];
    }


}
