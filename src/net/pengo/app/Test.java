/*
 * Test.java
 *
 * Created on 10 December 2004, 03:19
 */

package net.pengo.app;

import java.awt.BorderLayout;
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Rectangle;
import javax.swing.JPanel;

/**
 *
 * @author  Que
 */
public class Test extends javax.swing.JFrame {
    
    /** Creates new form Test */
    public Test() {
        initComponents();
        
        JPanel jp = new JPanel() {
            public void paint(Graphics g) {
                super.paint(g);
                paintComponents(g);
            }
            public void paintComponents(Graphics g) {
                super.paintComponents(g);
                Rectangle r = g.getClipBounds();
                g.drawString(r+"", 10,10);
                System.out.println(r);
                
                g.translate(100,100);
                
                r = g.getClipBounds();
                g.drawString(r+"", 10,10);
                System.out.println(r);                
            }
        };
        Dimension size = new Dimension(300,300);
        jp.setMinimumSize(size);
        jp.setMaximumSize(size);
        jp.setSize(size);
        getRootPane().setLayout(new BorderLayout());
        getRootPane().add(jp, BorderLayout.CENTER);
        pack();
    }
    
    /** This method is called from within the constructor to
     * initialize the form.
     * WARNING: Do NOT modify this code. The content of this method is
     * always regenerated by the Form Editor.
     */
    private void initComponents() {//GEN-BEGIN:initComponents
        
        setDefaultCloseOperation(javax.swing.WindowConstants.EXIT_ON_CLOSE);
        pack();
    }//GEN-END:initComponents
    
    /**
     * @param args the command line arguments
     */
    public static void main(String args[]) {
        java.awt.EventQueue.invokeLater(new Runnable() {
            public void run() {
                new Test().setVisible(true);
            }
        });
    }
    
    // Variables declaration - do not modify//GEN-BEGIN:variables
    // End of variables declaration//GEN-END:variables
    
}
