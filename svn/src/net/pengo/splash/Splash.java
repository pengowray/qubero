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

/*
 * Splash.java
 *
 * Created on 13 October 2002, 23:21
 */

package net.pengo.splash;

import javax.swing.*;
import java.awt.*;
import java.awt.event.*;

/**
 * the splash is a bit of a hack
 * is only used to get font sizes (used for window sizing or something)

 * @author  Peter Halasz
 */
public class Splash {
    private static boolean done = false;
    
    public static void show() {
        JFrame j = new JFrame("Qubero loading...") {
            
            public void paint(Graphics g) {
                FontMetricsCache.singleton().lendGraphics(g);
                done();
            }
            
            public void done() {
                setVisible(false);
                done = true;
                //FIXME: notifyAll() gives error
                //notifyAll();  //FIXME: use notify. i.e. make more specific.
            }


        };
        j.setSize(400,100);
        j.setVisible(true);
        
        // dont return until closed.
        while (!done) {
            try {
                Thread.sleep(100);
            } catch (InterruptedException e) {
                // will try again...
            }
        }
    }
    
    
}
