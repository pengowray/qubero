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

 * @author  administrator
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
