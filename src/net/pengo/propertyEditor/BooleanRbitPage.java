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

/**
 *
 * @author  Peter Halasz
 */

public class BooleanRbitPage extends EditablePage {
    private BooleanAddressedResource res;
    private JTextField inputField;
    
    public BooleanRbitPage(BooleanAddressedResource res, AbstractResourcePropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;
        
        //FIXME: really left?
        add(new JLabel( "Bit to use (0 for first on left<?>): " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
        
        JButton pointerButton = new JButton("...");
        
        pointerButton.addActionListener(new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                new ResourceSelectorForm(BooleanRbitPage.this.res.getRbitPointer()).show();
            }
        });
        
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

