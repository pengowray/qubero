/*
 * InputBox.java
 *
 * Created on 24 August 2002, 20:20
 */
package net.pengo.resource;


import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.io.IOException;

/**
 * in future: umm. this should be a lot more customizable.. give user a choice
 * of input styles (text, hex, binary, pointer, formula, immediate calculations)
 * and show the resulting value in a number of formats, or at least the type's
 * format.
 *
 * @author  administrator
 */
public class IntInputBox extends JFrame {
    protected final IntResource modRes; // modifiable resource.

    protected JLabel textLabel;
    protected JTextField inputField;
    protected JButton ok = new JButton("Okay");
    protected JButton cancel = new JButton("Cancel");
    
    /** Creates a new instance of InputBox */
    public IntInputBox(String title, String text, String defaultResult, IntResource res) {
        super(title);
        this.modRes = res;
        
        textLabel = new JLabel(text);
        inputField = new JTextField(defaultResult,12);
        Container cp = getContentPane();
        
        cp.setLayout(new FlowLayout()); // XXX lay out properly
        cp.add(textLabel);
        cp.add(inputField);
        cp.add(ok);
        cp.add(cancel);
        pack();
        
        AbstractAction okaction = new AbstractAction() {
            public void actionPerformed(ActionEvent e) {
                String retStr = inputField.getText();
                int retInt;
                try {
                    //retInt = Integer.parseInt(retStr);
                    //FIXME: pay attention to integer size and type!
                    modRes.setValue(retStr);
                    close();
                } catch (NumberFormatException nfe) {
                    //fixme:
		    System.out.println("Invalid!");
                } catch (IOException ioe) {
		    //fixme:
		    System.out.println("error!");
		    ioe.printStackTrace();
		}
            }
        };

        cancel.addActionListener(new AbstractAction() {
            public void actionPerformed(ActionEvent e) {
                close();
            }
        });
        ok.addActionListener(okaction);
        inputField.addActionListener(okaction);
    
    }
    
    protected void close() {
        //FIXME: ???
        setVisible(false);
    }
    
    
}
