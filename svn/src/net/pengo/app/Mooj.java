/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

package net.pengo.app;

import javax.swing.UIManager;
import javax.swing.UIManager.LookAndFeelInfo;
import javax.swing.UnsupportedLookAndFeelException;
import net.pengo.data.Data;
import net.pengo.data.LargeFileData;
import net.pengo.splash.Splash;

public class Mooj {
    public static void main(String[] arg) {
            
            
            
            try {

                System.out.println("max: " + Byte.MAX_VALUE );
	    //changeLookAndFeel();
            
	    Splash.show();
	    if (arg.length == 0) {
		////arg = new String[] {"C:\\My Documents\\project-moojasm\\HexEditorGUI.class"}; // Mooj.class
		//rd = new DemoFileChunk();
		//OpenFile of = new OpenFile(rd);
		//new HexEditorGUI(of);
		new GUI();

	        //throw new StackOverflowError();
		
	    } else {
		Data d = new LargeFileData(arg[0]);
		new GUI(d);
	    }
	    
	} catch (java.lang.StackOverflowError e) {
	    e.printStackTrace();
	}
    }

    /** changes the look and feel so it is NOT the default */
    static void changeLookAndFeel() {
        try {
            //UIManager ui = new UIManager();
            LookAndFeelInfo[] laf = UIManager.getInstalledLookAndFeels();
            if (laf == null) {
                System.out.println("note: UIManager.getInstalledLookAndFeels() returns null");
                return;
            }
            
            String defaultLAF = UIManager.getLookAndFeel().getName();
            System.out.println("default look and feel name: " + defaultLAF);
            System.out.println("installed look and feels: " + laf.length);
            
            LookAndFeelInfo l2 = null;
            for (LookAndFeelInfo l : laf) {
                l2 = l;
                System.out.println("candidate look and feel name: " + l.getName());
                if (! l.getName().equals(defaultLAF)) {
                    UIManager.setLookAndFeel(l2.getClassName());
                    System.out.println(l.getName() + " is not default");
                    return;
                }

            }
            System.out.println("setting to: " + l2.getName() + ".");
            UIManager.setLookAndFeel(l2.getClassName());
            System.out.println("no other auxilary l+f found :(");

        } catch (ClassNotFoundException e) {
            e.printStackTrace();
        } catch (InstantiationException e) {
            e.printStackTrace();
        } catch (IllegalAccessException e) {
            e.printStackTrace();
        } catch (UnsupportedLookAndFeelException e) {
            e.printStackTrace();
        }
    }
    
}



/**
 * A view contains information on how to display chunks, options and commands
 * in a USEFUL manner -- hiding redundancy and triviality.
 *
 * FIXME: In future, there should be only a handful of generic views, which will be
 * user-customisable and dynamic. e.g. "zoom" level of detail. Perhaps file format specific
 * views will become necessary also.
 */

//class View {
//
//}


/**
 * This presentation displays only hexidecimal values.
 */
//class PlainHexView extends View {
//
//}

/**
 * This presentation favours displaying hexidecimal values over "friendlier" formats.
 */
//class HexView {
//
//}

/**
 * This presentation favours displaying high level information over lower level (eg hex) formats.
 */
//class HighView {
//
//}

/**
 * Display multiple levels: formatted for printing.
 */
//class PrintView {
//
//}

/**
 * In the debug view, everything is shown.
 */
//class DebugView extends View {
//
//}

/**
 * Provides the rendering engines which do the dirty work.
 * class RendererFactory {
 *
 * }
 */
