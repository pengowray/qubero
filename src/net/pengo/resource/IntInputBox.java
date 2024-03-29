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
 * InputBox.java
 *
 * Created on 24 August 2002, 20:20
 */
package net.pengo.resource;


import java.awt.Container;
import java.awt.FlowLayout;
import java.awt.event.ActionEvent;
import java.io.IOException;

import javax.swing.AbstractAction;
import javax.swing.JButton;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JTextField;

/**
 * in future: umm. this should be a lot more customizable.. give user a choice
 * of input styles (text, hex, binary, pointer, formula, immediate calculations)
 * and show the resulting value in a number of formats, or at least the type's
 * format.
 *
 * @author  Peter Halasz
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
