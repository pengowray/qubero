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

/**
 * BooleanRbitPage.java
 *
 
 */

package net.pengo.propertyEditor;

import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;
import java.io.IOException;

import javax.swing.JButton;
import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.pointer.JavaPointer;
import net.pengo.resource.BooleanAddressedResource;
import net.pengo.resource.ResourceSelectorForm;

/**
 *
 * @author  Peter Halasz
 */

public class BooleanRbitPage extends EditablePage {
    private BooleanAddressedResource res;
    private JTextField inputField;
    
    public BooleanRbitPage(BooleanAddressedResource res, PropertiesForm form) {
        super(form);
        this.res = res;
        
        //FIXME: really left?
        add(new JLabel( "Bit to use (0 for first on left<?>): " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
        
        JButton pointerButton = new JButton("...");
        //FIXME: "..."
        /*
        pointerButton.addActionListener(new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                new ResourceSelectorForm(BooleanRbitPage.this.res.getRbitPointer()).show();
            }
        });
        */
        
        add(inputField);
        add(pointerButton);
        build();
    }
    
    
    public void saveOp() {
        if (isOwner()) {
            try {
                res.getRbit().setValue(inputField.getText());
            } catch (IOException e ) {
                //fixme
                e.printStackTrace();
            }
        } else {
            // it's already set then
        }
    }
    
    public void buildOp() {
        if (isOwner()) {
            inputField.setText( res.getRbit().getValue() + "" );
            inputField.setEnabled(true);
        } else {
            //fixme: give more info on the pointer
            inputField.setText( res.getRbit().getValue() + "" );
            inputField.setEnabled(false);
        }
    }
    
    /** is the rbitpointer the owner of its pointed-to-thing? */
    private boolean isOwner() {
        JavaPointer rbp = res.getRbitPointer();
        boolean owner = rbp.getPrimarySource().isOwner(rbp);
        return owner;
    }
    
    public boolean isValid() {
        //fixme.. also fix IntValuePage
        return true; // success
    }
    
    public String toString() {
        return "Value";
    }
}

